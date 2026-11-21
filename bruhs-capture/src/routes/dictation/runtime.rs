//! The inference thread owns its Whisper context. Settings changes wake it even
//! between recordings so an idle deadline never depends on the next request.
use super::*;
use crate::transcribe::whisper::PersistentContext;
use bruhs_models::dictation_runtime::{Change, Hardware, Settings, Status};
use std::{
    io,
    path::{Path, PathBuf},
    sync::{mpsc, Mutex},
    time::{Duration, Instant},
};
use tokio::sync::{oneshot, SemaphorePermit};

type HttpError = (StatusCode, String);
struct Job {
    path: PathBuf,
    model: bruhs_models::WhisperModel,
    samples: Vec<f32>,
    language: Option<String>,
    _permit: SemaphorePermit<'static>,
    reply: oneshot::Sender<Result<String, String>>,
}
enum Message {
    PreferencesChanged,
    Release(bruhs_models::WhisperModel, oneshot::Sender<()>),
    Transcribe(Job),
}
struct Resident {
    model: bruhs_models::WhisperModel,
    path: PathBuf,
    hardware: Hardware,
    used: Instant,
    context: PersistentContext,
}

pub struct Engine {
    file: PathBuf,
    settings: Arc<Mutex<Settings>>,
    loaded: Arc<Mutex<Option<bruhs_models::WhisperModel>>>,
    worker: Mutex<Option<mpsc::Sender<Message>>>,
    gpu: std::sync::OnceLock<Option<String>>,
}
fn lock<T>(state: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    state
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}
impl Engine {
    pub fn new(file: PathBuf) -> Self {
        let settings = match std::fs::read(&file) {
            Ok(data) => serde_json::from_slice(&data).unwrap_or_else(|e| {
                tracing::warn!(%e,"Could not read dictation preferences");
                Default::default()
            }),
            Err(e) => {
                if e.kind() != io::ErrorKind::NotFound {
                    tracing::warn!(%e,"Could not open dictation preferences");
                }
                Default::default()
            }
        };
        Self {
            file,
            settings: Arc::new(Mutex::new(settings)),
            loaded: Default::default(),
            worker: Default::default(),
            gpu: Default::default(),
        }
    }
    fn gpu(&self) -> Option<String> {
        self.gpu
            .get_or_init(|| {
                #[cfg(target_os = "macos")]
                {
                    cidre::mtl::Device::sys_default().map(|d| d.name().to_string())
                }
                #[cfg(not(target_os = "macos"))]
                {
                    None
                }
            })
            .clone()
    }
    fn status(&self, compatible_origin: Option<String>) -> Status {
        Status {
            settings: *lock(&self.settings),
            gpu: self.gpu(),
            loaded: *lock(&self.loaded),
            compatible_origin,
        }
    }
    fn configure(&self, change: Change) -> Result<(), HttpError> {
        {
            let mut guard = lock(&self.settings);
            let mut next = *guard;
            match change {
                Change::Hardware(Hardware::Gpu) if self.gpu().is_none() => {
                    return Err((
                        StatusCode::BAD_REQUEST,
                        "This host has no supported transcription GPU.".into(),
                    ))
                }
                Change::Hardware(hardware) => next.hardware = hardware,
                Change::Unload(unload) => next.unload = unload,
            }
            let persist = || -> io::Result<()> {
                if let Some(parent) = self.file.parent() {
                    std::fs::create_dir_all(parent)?;
                }
                crate::paths::write_private_atomic(&self.file, &serde_json::to_vec(&next)?)
            };
            persist().map_err(|e| {
                (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    format!("Could not save dictation preferences: {e}"),
                )
            })?;
            *guard = next;
        }
        if let Some(worker) = lock(&self.worker).as_ref() {
            let _ = worker.send(Message::PreferencesChanged);
        }
        Ok(())
    }
    pub(super) async fn release(&self, model: bruhs_models::WhisperModel) {
        let (reply, done) = oneshot::channel();
        let sent = lock(&self.worker)
            .as_ref()
            .is_some_and(|worker| worker.send(Message::Release(model, reply)).is_ok());
        if sent {
            let _ = done.await;
        }
    }
    fn submit(&self, job: Job) -> Result<(), String> {
        let mut worker = lock(&self.worker);
        if worker.is_none() {
            let (send, recv) = mpsc::channel();
            let settings = self.settings.clone();
            let loaded = self.loaded.clone();
            let gpu = self.gpu().is_some();
            std::thread::Builder::new()
                .name("bruhs-dictation".into())
                .spawn(move || run(recv, &settings, &loaded, gpu))
                .map_err(|e| e.to_string())?;
            *worker = Some(send);
        }
        if worker
            .as_ref()
            .is_some_and(|w| w.send(Message::Transcribe(job)).is_ok())
        {
            return Ok(());
        }
        *worker = None;
        Err("The dictation worker stopped.".into())
    }
}
fn run(
    messages: mpsc::Receiver<Message>,
    settings: &Mutex<Settings>,
    loaded: &Mutex<Option<bruhs_models::WhisperModel>>,
    gpu: bool,
) {
    let mut resident: Option<Resident> = None;
    loop {
        let prefs = *lock(settings);
        if resident.as_ref().is_some_and(|r| {
            r.hardware != prefs.hardware
                || prefs
                    .unload
                    .seconds()
                    .is_some_and(|s| r.used.elapsed() >= Duration::from_secs(s))
        }) {
            resident = None;
            *lock(loaded) = None;
        }
        let remaining = resident.as_ref().and_then(|r| {
            prefs
                .unload
                .seconds()
                .map(|s| Duration::from_secs(s).saturating_sub(r.used.elapsed()))
        });
        let message = if let Some(timeout) = remaining {
            match messages.recv_timeout(timeout) {
                Ok(m) => m,
                Err(mpsc::RecvTimeoutError::Timeout) => continue,
                Err(mpsc::RecvTimeoutError::Disconnected) => break,
            }
        } else {
            let Ok(m) = messages.recv() else {
                break;
            };
            m
        };
        let job = match message {
            Message::Transcribe(job) => job,
            Message::PreferencesChanged => continue,
            Message::Release(model, reply) => {
                if resident.as_ref().is_some_and(|r| r.model == model) {
                    resident = None;
                    *lock(loaded) = None;
                }
                let _ = reply.send(());
                continue;
            }
        };
        let hardware = lock(settings).hardware;
        let result = (|| {
            if !resident.as_ref().is_some_and(|r| {
                r.path == job.path && r.model == job.model && r.hardware == hardware
            }) {
                // Release the previous model before allocating another large context.
                resident = None;
                *lock(loaded) = None;
                let use_gpu = hardware != Hardware::Cpu && gpu;
                let context = PersistentContext::with_hardware(&job.path, use_gpu)
                    .map_err(|e| e.to_string())?;
                resident = Some(Resident {
                    context,
                    model: job.model,
                    path: job.path.clone(),
                    hardware,
                    used: Instant::now(),
                });
                *lock(loaded) = Some(job.model);
            }
            let r = resident
                .as_mut()
                .ok_or("The dictation model did not load.")?;
            let text = r
                .context
                .transcribe(&job.samples, job.language.as_deref(), |_| {})
                .map_err(|e| e.to_string())?
                .into_iter()
                .map(|segment| segment.text)
                .collect::<Vec<_>>()
                .join(" ");
            r.used = Instant::now();
            Ok::<_, String>(text)
        })();
        if result.is_err() || lock(settings).unload.seconds() == Some(0) {
            resident = None;
            *lock(loaded) = None;
        }
        drop(job._permit);
        let _ = job.reply.send(result);
    }
    *lock(loaded) = None;
}
pub async fn get(State(state): State<Arc<crate::state::CaptureState>>) -> Json<Status> {
    Json(state.dictation.status(state.remote_transcription.origin()))
}
pub async fn set(
    State(state): State<Arc<crate::state::CaptureState>>,
    Json(change): Json<Change>,
) -> Result<Json<Status>, HttpError> {
    tokio::task::spawn_blocking(move || {
        state.dictation.configure(change)?;
        Ok(state.dictation.status(state.remote_transcription.origin()))
    })
    .await
    .map_err(internal)?
    .map(Json)
}
pub(super) async fn infer(
    engine: &Engine,
    path: &Path,
    model: bruhs_models::WhisperModel,
    samples: Vec<f32>,
    language: Option<String>,
    permit: SemaphorePermit<'static>,
) -> Result<String, HttpError> {
    let (reply, receive) = oneshot::channel();
    engine
        .submit(Job {
            path: path.into(),
            model,
            samples,
            language,
            _permit: permit,
            reply,
        })
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e))?;
    receive
        .await
        .map_err(|_| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                "The dictation worker stopped.".into(),
            )
        })?
        .map_err(|e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("Local dictation failed: {e}"),
            )
        })
}
