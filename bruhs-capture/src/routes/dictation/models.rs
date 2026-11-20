//! Explicit downloads share the verified model store with ordinary recordings.
use super::*;
use crate::{models_dir::ModelArtifact, state::CaptureState};
use bruhs_models::{
    dictation_runtime::{ModelAction, ModelEntry},
    WhisperModel,
};
use std::{collections::HashMap, sync::Mutex};
use tokio::sync::oneshot;

#[derive(Default)]
struct Transfer {
    received: Option<u64>,
    cancel: Option<oneshot::Sender<()>>,
    error: Option<String>,
}
#[derive(Default)]
pub struct Catalog {
    transfers: Mutex<HashMap<&'static str, Transfer>>,
    changes: tokio::sync::Mutex<()>,
}
impl Catalog {
    fn lock(&self) -> std::sync::MutexGuard<'_, HashMap<&'static str, Transfer>> {
        self.transfers
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }
}

async fn inventory(state: &CaptureState) -> Vec<ModelEntry> {
    let mut rows = Vec::new();
    for &model in WhisperModel::all() {
        let artifact = ModelArtifact::whisper(model);
        let installed = tokio::fs::metadata(state.models.path(&artifact))
            .await
            .is_ok_and(|meta| meta.is_file() && meta.len() == artifact.len);
        let transfers = state.dictation_models.lock();
        let transfer = transfers.get(model.key());
        rows.push(ModelEntry {
            model,
            installed,
            downloaded_bytes: transfer.and_then(|t| t.received),
            error: transfer.and_then(|t| t.error.clone()),
        });
    }
    rows
}
pub async fn get(State(state): State<Arc<CaptureState>>) -> Json<Vec<ModelEntry>> {
    Json(inventory(&state).await)
}
fn download(state: Arc<CaptureState>, model: WhisperModel) {
    let (cancel, mut stop) = oneshot::channel();
    {
        let mut entries = state.dictation_models.lock();
        let entry = entries.entry(model.key()).or_default();
        if entry.received.is_some() {
            return;
        }
        *entry = Transfer {
            received: Some(0),
            cancel: Some(cancel),
            error: None,
        };
    }
    tokio::spawn(async move {
        let result = tokio::select! {
            biased;
            _ = &mut stop => Ok(()),
            result = state.models.ensure_whisper(model, |bytes, _| {
                if let Some(transfer) = state.dictation_models.lock().get_mut(model.key()) {
                    transfer.received = Some(bytes);
                }
            }) => result.map(|_| ()),
        };
        *state
            .dictation_models
            .lock()
            .entry(model.key())
            .or_default() = Transfer {
            error: result.err().map(|error| error.to_string()),
            ..Default::default()
        };
    });
}
pub async fn change(
    State(state): State<Arc<CaptureState>>,
    Json(action): Json<ModelAction>,
) -> Result<Json<Vec<ModelEntry>>, (StatusCode, String)> {
    let _change = state.dictation_models.changes.lock().await;
    match action {
        ModelAction::Download(model) => download(state.clone(), model),
        ModelAction::Import { model, path } => {
            if state
                .dictation_models
                .lock()
                .get(model.key())
                .is_some_and(|entry| entry.received.is_some())
            {
                return Err((
                    StatusCode::CONFLICT,
                    "Finish or cancel the model download first.".into(),
                ));
            }
            if path.len() > 4096 {
                return Err((
                    StatusCode::BAD_REQUEST,
                    "The model path is too long.".into(),
                ));
            }
            state
                .models
                .import_whisper(model, path.into())
                .await
                .map_err(|e| (StatusCode::UNPROCESSABLE_ENTITY, e.to_string()))?;
            state.dictation_models.lock().remove(model.key());
        }
        ModelAction::Cancel(model) => {
            if let Some(cancel) = state
                .dictation_models
                .lock()
                .get_mut(model.key())
                .and_then(|e| e.cancel.take())
            {
                let _ = cancel.send(());
            }
        }
        ModelAction::Remove(model) => {
            let busy = || {
                (
                    StatusCode::CONFLICT,
                    "The model is in use. Finish the recording or download and try again."
                        .to_owned(),
                )
            };
            let _dictation = INFERENCE.try_acquire().map_err(|_| busy())?;
            let _transition = state.lifecycle.try_enter().ok_or_else(busy)?;
            let _meetings = state
                .models
                .meeting_inference
                .try_write()
                .map_err(|_| busy())?;
            if state.with_current(|session| session.is_some())
                || state
                    .dictation_models
                    .lock()
                    .get(model.key())
                    .is_some_and(|e| e.received.is_some())
            {
                return Err(busy());
            }
            state.dictation.release(model).await;
            state
                .models
                .remove_whisper(model)
                .await
                .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
        }
    }
    Ok(Json(inventory(&state).await))
}
