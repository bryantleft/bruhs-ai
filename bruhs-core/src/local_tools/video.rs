//! Continuous device capture. The pipeline owns recorder/transcoder groups from
//! their first spawn, including setup failures; frames never enter durable storage.
use super::{devices, LocalToolsHost, ProcessPolicy, ToolError};
use bruhs_models::local_tools::{DevicePlatform, DeviceVideoStart, ToolScope};
use futures_util::{stream::FuturesUnordered, StreamExt};
use std::{
    process::{ExitStatus, Stdio},
    sync::{Arc, Mutex, Weak},
    time::Duration,
};
use tokio::{
    io::{AsyncBufRead, AsyncBufReadExt, AsyncRead, BufReader},
    process::{Child, ChildStdout, Command},
    sync::watch,
    task::JoinHandle,
};

// Match the shared CLI wrapper's inherited-stderr settle deadline.
const STDERR_SETTLE: Duration = Duration::from_secs(1);

struct Slot {
    host: Weak<LocalToolsHost>,
    grant: String,
}
impl Drop for Slot {
    fn drop(&mut self) {
        if let Some(host) = self.host.upgrade() {
            if let Ok(mut slots) = host.video.lock() {
                slots.remove(&self.grant);
            }
        }
    }
}

#[cfg(unix)]
struct ProcessGroup(Option<i32>);
#[cfg(unix)]
impl ProcessGroup {
    fn stop(&mut self) {
        if let Some(pid) = self.0.take() {
            // SAFETY: this positive ID is from a child we just spawned in a
            // fresh group. No saved PID or pre-existing companion is targeted.
            unsafe {
                libc::kill(-pid, libc::SIGKILL);
            }
        }
    }
}
#[cfg(unix)]
impl Drop for ProcessGroup {
    fn drop(&mut self) {
        self.stop();
    }
}

struct CaptureChild {
    name: String,
    child: Option<Child>,
    #[cfg(unix)]
    group: ProcessGroup,
    stderr: Option<JoinHandle<()>>,
    diagnostic: Arc<Mutex<Vec<u8>>>,
    exited: Option<ExitStatus>,
}
impl CaptureChild {
    fn failure(&self, status: ExitStatus) -> ToolError {
        let detail = self
            .diagnostic
            .lock()
            .map(|bytes| String::from_utf8_lossy(&bytes).into_owned())
            .unwrap_or_default();
        ToolError::Unavailable(format!("Device video {} failed ({status}): {detail}. Check the device and installed capture tools, then restart video.", self.name))
    }

    async fn wait(&mut self) -> Result<ExitStatus, ToolError> {
        let child = self.child.as_mut().ok_or_else(|| {
            ToolError::Unavailable(
                "The owned capture process is unavailable. Restart video.".into(),
            )
        })?;
        let status = child.wait().await?;
        #[cfg(unix)]
        self.group.stop();
        if let Some(worker) = self.stderr.as_mut() {
            if tokio::time::timeout(STDERR_SETTLE, &mut *worker)
                .await
                .is_err()
            {
                worker.abort();
            }
        }
        self.stderr.take();
        self.exited = Some(status);
        Ok(status)
    }
}
impl Drop for CaptureChild {
    fn drop(&mut self) {
        if let Some(worker) = self.stderr.take() {
            worker.abort();
        }
        #[cfg(unix)]
        self.group.stop();
        if let Some(mut child) = self.child.take() {
            let _ = child.start_kill();
            if let Ok(runtime) = tokio::runtime::Handle::try_current() {
                runtime.spawn(async move {
                    let _ = child.wait().await;
                });
            }
        }
    }
}

#[derive(Default)]
struct Pipeline {
    children: Vec<CaptureChild>,
    workers: Vec<JoinHandle<()>>,
}
impl Drop for Pipeline {
    fn drop(&mut self) {
        for worker in &self.workers {
            worker.abort();
        }
        // CaptureChild kills its fresh process group and reaps its direct child.
        self.children.clear();
    }
}
impl Pipeline {
    fn spawn(
        &mut self,
        program: &str,
        args: &[String],
        input: bool,
        policy: ProcessPolicy,
    ) -> Result<ChildStdout, ToolError> {
        let mut command = Command::new(program);
        command
            .args(args)
            .env("PYTHONUNBUFFERED", "1")
            .stdin(if input { Stdio::piped() } else { Stdio::null() })
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true);
        #[cfg(unix)]
        command.process_group(0);
        let child = command.spawn().map_err(|error| ToolError::Unavailable(format!("Cannot start device video using {program}: {error}. Android needs adb and ffmpeg; iOS needs the idb CLI and companion on Bruhs' PATH.")))?;
        #[cfg(unix)]
        let group = ProcessGroup(child.id().and_then(|id| i32::try_from(id).ok()));
        let mut owned = CaptureChild {
            name: program.into(),
            child: Some(child),
            #[cfg(unix)]
            group,
            stderr: None,
            diagnostic: Arc::new(Mutex::new(Vec::new())),
            exited: None,
        };
        let child = owned
            .child
            .as_mut()
            .ok_or_else(|| ToolError::Unavailable("The capture process is unavailable.".into()))?;
        let output = child.stdout.take().ok_or_else(|| {
            ToolError::Unavailable(format!("{program} has no video output pipe."))
        })?;
        if let Some(stderr) = child.stderr.take() {
            let (worker, diagnostic) = drain(stderr, policy.max_output_bytes);
            owned.stderr = Some(worker);
            owned.diagnostic = diagnostic;
        }
        self.children.push(owned);
        Ok(output)
    }

    fn pipe_to_last(&mut self, mut source: ChildStdout) -> Result<(), ToolError> {
        let mut input = self
            .children
            .last_mut()
            .and_then(|owned| owned.child.as_mut())
            .and_then(|child| child.stdin.take())
            .ok_or_else(|| {
                ToolError::Unavailable("The capture transcoder has no input pipe.".into())
            })?;
        self.workers.push(tokio::spawn(async move {
            let _ = tokio::io::copy(&mut source, &mut input).await;
        }));
        Ok(())
    }
}

pub struct DeviceVideo {
    reader: BufReader<ChildStdout>,
    pipeline: Pipeline,
    revoked: watch::Receiver<bool>,
    policy: ProcessPolicy,
    first: Option<Vec<u8>>,
    _slot: Slot,
}
impl DeviceVideo {
    pub async fn next_frame(&mut self) -> Result<Option<Vec<u8>>, ToolError> {
        if *self.revoked.borrow() {
            return Err(ToolError::Revoked);
        }
        if let Some(frame) = self.first.take() {
            return Ok(Some(frame));
        }
        let terminal = self.pipeline.children.len().checked_sub(1);
        let terminal_exited = self
            .pipeline
            .children
            .last()
            .is_some_and(|child| child.exited.is_some());
        for child in &self.pipeline.children {
            if let Some(status) = child.exited.filter(|status| !status.success()) {
                return Err(child.failure(status));
            }
        }
        let read = async {
            let mut terminal_exited = terminal_exited;
            let mut exits = self
                .pipeline
                .children
                .iter_mut()
                .enumerate()
                .filter(|(_, child)| child.exited.is_none())
                .map(|(index, child)| async move {
                    let status = child.wait().await?;
                    if !status.success() {
                        return Err(child.failure(status));
                    }
                    Ok::<_, ToolError>((index, status))
                })
                .collect::<FuturesUnordered<_>>();
            let frame = jpeg(&mut self.reader, self.policy.max_output_bytes);
            tokio::pin!(frame);
            let mut ended = false;
            loop {
                if ended && (terminal_exited || exits.is_empty()) {
                    return Ok(None);
                }
                tokio::select! {
                    biased;
                    status = exits.next(), if !exits.is_empty() => {
                        if let Some(status) = status {
                            let (index, _) = status?;
                            terminal_exited |= Some(index) == terminal;
                        }
                    }
                    result = &mut frame, if !ended => match result? {
                        Some(frame) => return Ok(Some(frame)),
                        None => ended = true,
                    }
                }
            }
        };
        tokio::select! {
            biased;
            _ = self.revoked.changed() => Err(ToolError::Revoked),
            frame = tokio::time::timeout(self.policy.timeout, read) => {
                frame.map_err(|_| ToolError::Unavailable(format!("Device video produced no complete frame within {:?}. Check the device and restart video.", self.policy.timeout)))?
            }
        }
    }
}

fn drain(
    stderr: impl AsyncRead + Unpin + Send + 'static,
    max: usize,
) -> (JoinHandle<()>, Arc<Mutex<Vec<u8>>>) {
    let diagnostic = Arc::new(Mutex::new(Vec::new()));
    let output = diagnostic.clone();
    let worker = tokio::spawn(async move {
        let mut reader = BufReader::new(stderr);
        while let Ok(bytes) = reader.fill_buf().await {
            if bytes.is_empty() {
                break;
            }
            if let Ok(mut retained) = output.lock() {
                let length = bytes.len().min(max.saturating_sub(retained.len()));
                retained.extend_from_slice(&bytes[..length]);
            }
            let length = bytes.len();
            reader.consume(length);
        }
    });
    (worker, diagnostic)
}

pub(super) async fn start(
    host: &Arc<LocalToolsHost>,
    request: DeviceVideoStart,
) -> Result<DeviceVideo, ToolError> {
    let mut revoked = host.require(&request.grant_id, ToolScope::Devices)?;
    {
        let mut slots = host.video.lock().map_err(|_| ToolError::Revoked)?;
        if !slots.insert(request.grant_id.clone()) {
            return Err(ToolError::Invalid("This device grant already has live video. Stop that stream before starting another.".into()));
        }
    }
    let slot = Slot {
        host: Arc::downgrade(host),
        grant: request.grant_id,
    };
    let inventory = tokio::select! {
        biased;
        _ = revoked.changed() => return Err(ToolError::Revoked),
        inventory = devices::inventory(host.policy) => inventory?,
    };
    let device = inventory
        .devices
        .iter()
        .find(|device| device.platform == request.platform && device.id == request.device_id)
        .ok_or_else(|| {
            ToolError::Invalid(
                "Refresh Devices and select a connected device before starting video.".into(),
            )
        })?;
    if (device.platform == DevicePlatform::Android && device.state != "device")
        || (device.platform == DevicePlatform::Ios && device.state != "Booted")
    {
        return Err(ToolError::Unavailable(format!(
            "{} is {}. Boot or authorize it before starting video.",
            device.name, device.state
        )));
    }
    let mut pipeline = Pipeline::default();
    let output = match request.platform {
        DevicePlatform::Android => {
            let mut args = devices::android_args(
                &device.id,
                &bruhs_models::local_tools::DeviceAction::Screenshot,
            )?;
            args.truncate(2);
            args.extend(
                ["exec-out", "screenrecord", "--output-format=h264", "-"].map(String::from),
            );
            let source = pipeline.spawn("adb", &args, false, host.policy)?;
            let output = pipeline.spawn(
                "ffmpeg",
                &[
                    "-hide_banner",
                    "-loglevel",
                    "error",
                    "-f",
                    "h264",
                    "-i",
                    "pipe:0",
                    "-an",
                    "-f",
                    "image2pipe",
                    "-c:v",
                    "mjpeg",
                    "pipe:1",
                ]
                .map(String::from),
                true,
                host.policy,
            )?;
            pipeline.pipe_to_last(source)?;
            output
        }
        DevicePlatform::Ios => {
            uuid::Uuid::parse_str(&device.id)
                .map_err(|_| ToolError::Invalid("Invalid iOS simulator identifier.".into()))?;
            pipeline.spawn(
                "idb",
                &[
                    "video-stream".into(),
                    "--format".into(),
                    "mjpeg".into(),
                    "--udid".into(),
                    device.id.clone(),
                ],
                false,
                host.policy,
            )?
        }
    };
    let mut video = DeviceVideo {
        reader: BufReader::new(output),
        pipeline,
        revoked,
        policy: host.policy,
        first: None,
        _slot: slot,
    };
    let first = video.next_frame().await?.ok_or_else(|| ToolError::Unavailable("Device recorder ended before its first frame. Check adb/ffmpeg or idb and its companion, then retry.".into()))?;
    video.first = Some(first);
    Ok(video)
}

async fn jpeg(
    reader: &mut (impl AsyncBufRead + Unpin),
    max: usize,
) -> Result<Option<Vec<u8>>, ToolError> {
    let mut frame = Vec::new();
    let mut previous_ff = false;
    let mut started = false;
    let mut skipped = 0usize;
    loop {
        let bytes = reader.fill_buf().await?;
        if bytes.is_empty() {
            return if frame.is_empty() {
                Ok(None)
            } else {
                Err(ToolError::Unavailable(
                    "Device video ended during a JPEG frame. Restart video.".into(),
                ))
            };
        }
        let mut start = started.then_some(0);
        for (index, byte) in bytes.iter().copied().enumerate() {
            if !started {
                skipped = skipped.saturating_add(1);
                if skipped > max {
                    return Err(ToolError::Unavailable(
                        "Device video did not contain a JPEG frame within the capture budget."
                            .into(),
                    ));
                }
                if previous_ff && byte == 0xd8 {
                    started = true;
                    frame.push(0xff);
                    start = Some(index);
                }
            } else if previous_ff && byte == 0xd9 {
                let begin = start.unwrap_or(0);
                if frame.len().saturating_add(index + 1 - begin) > max {
                    return Err(ToolError::Unavailable("A device video frame exceeded the capture budget. Reduce device resolution and restart video.".into()));
                }
                frame.extend_from_slice(&bytes[begin..=index]);
                reader.consume(index + 1);
                return Ok(Some(frame));
            }
            previous_ff = byte == 0xff;
        }
        if let Some(start) = start {
            if frame.len().saturating_add(bytes.len() - start) > max {
                return Err(ToolError::Unavailable("A device video frame exceeded the capture budget. Reduce device resolution and restart video.".into()));
            }
            frame.extend_from_slice(&bytes[start..]);
        }
        let length = bytes.len();
        reader.consume(length);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[tokio::test]
    async fn continuous_stream_preserves_frames_across_read_boundaries() {
        let bytes: &[u8] = &[0, 0xff, 0xd8, 1, 2, 0xff, 0xd9, 0xff, 0xd8, 3, 0xff, 0xd9];
        let mut reader = BufReader::with_capacity(1, bytes);
        assert_eq!(
            jpeg(&mut reader, 16).await.unwrap().unwrap(),
            [0xff, 0xd8, 1, 2, 0xff, 0xd9]
        );
        assert_eq!(
            jpeg(&mut reader, 16).await.unwrap().unwrap(),
            [0xff, 0xd8, 3, 0xff, 0xd9]
        );
        assert!(jpeg(&mut reader, 16).await.unwrap().is_none());
    }
    #[tokio::test]
    async fn oversized_and_truncated_frames_fail_explicitly() {
        assert!(jpeg(&mut BufReader::new(&b"\xff\xd8large\xff\xd9"[..]), 5)
            .await
            .is_err());
        assert!(jpeg(&mut BufReader::new(&b"\xff\xd8partial"[..]), 50)
            .await
            .is_err());
        assert!(jpeg(&mut BufReader::new(&b"no jpeg here"[..]), 5)
            .await
            .is_err());
    }
    #[tokio::test]
    async fn a_video_slot_is_released_on_failure_and_revocation_blocks_launch() {
        let host = LocalToolsHost::new().unwrap();
        let grant = host
            .grant(bruhs_models::local_tools::GrantRequest {
                principal: "video-test".into(),
                scope: ToolScope::Devices,
            })
            .unwrap();
        {
            host.video.lock().unwrap().insert(grant.id.clone());
            let _slot = Slot {
                host: Arc::downgrade(&host),
                grant: grant.id.clone(),
            };
            assert!(host
                .start_device_video(DeviceVideoStart {
                    grant_id: grant.id.clone(),
                    device_id: "unused".into(),
                    platform: DevicePlatform::Android
                })
                .await
                .is_err());
        }
        assert!(host.video.lock().unwrap().is_empty());
        host.revoke(&grant.id).await.unwrap();
        assert!(matches!(
            host.start_device_video(DeviceVideoStart {
                grant_id: grant.id,
                device_id: "unused".into(),
                platform: DevicePlatform::Android
            })
            .await,
            Err(ToolError::Revoked)
        ));
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn revocation_and_dropped_connection_stop_and_reap_owned_recorders() {
        for revoke in [false, true] {
            let host = LocalToolsHost::new().unwrap();
            let grant = host
                .grant(bruhs_models::local_tools::GrantRequest {
                    principal: format!("video-{revoke}"),
                    scope: ToolScope::Devices,
                })
                .unwrap();
            host.video.lock().unwrap().insert(grant.id.clone());
            let mut pipeline = Pipeline::default();
            let output = pipeline.spawn("/usr/bin/python3", &["-u".into(), "-c".into(), "import sys,time;sys.stdout.buffer.write(bytes([255,216,1,255,217,255,216,2,255,217]));sys.stdout.buffer.flush();time.sleep(3600)".into()], false, host.policy).unwrap();
            let pid = pipeline.children[0].child.as_ref().unwrap().id().unwrap();
            let mut video = DeviceVideo {
                reader: BufReader::new(output),
                pipeline,
                revoked: host.require(&grant.id, ToolScope::Devices).unwrap(),
                policy: host.policy,
                first: None,
                _slot: Slot {
                    host: Arc::downgrade(&host),
                    grant: grant.id.clone(),
                },
            };
            assert!(video.next_frame().await.unwrap().is_some());
            assert!(video.next_frame().await.unwrap().is_some());
            if revoke {
                host.revoke(&grant.id).await.unwrap();
                assert!(matches!(video.next_frame().await, Err(ToolError::Revoked)));
            }
            drop(video);
            tokio::time::timeout(host.policy.timeout, async {
                loop {
                    // SAFETY: signal zero only observes this test-owned process.
                    if unsafe { libc::kill(i32::try_from(pid).unwrap(), 0) } != 0 {
                        break;
                    }
                    tokio::task::yield_now().await;
                }
            })
            .await
            .expect("dropping video must kill and reap the recorder");
            assert!(host.video.lock().unwrap().is_empty());
        }
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn failed_transcoder_reports_its_bounded_diagnostic_while_the_recorder_is_alive() {
        let host = LocalToolsHost::new().unwrap();
        let grant = host
            .grant(bruhs_models::local_tools::GrantRequest {
                principal: "transcoder-failure".into(),
                scope: ToolScope::Devices,
            })
            .unwrap();
        host.video.lock().unwrap().insert(grant.id.clone());
        let mut pipeline = Pipeline::default();
        let source = pipeline
            .spawn(
                "/usr/bin/python3",
                &[
                    "-u".into(),
                    "-c".into(),
                    "import time;time.sleep(3600)".into(),
                ],
                false,
                host.policy,
            )
            .unwrap();
        let recorder = pipeline.children[0].child.as_ref().unwrap().id().unwrap();
        let diagnostic_budget = bruhs_models::mobile::TEXT_BYTES;
        let policy = ProcessPolicy {
            max_output_bytes: diagnostic_budget,
            ..host.policy
        };
        let output = pipeline.spawn("/usr/bin/python3", &["-u".into(), "-c".into(), "import sys;sys.stderr.write('fixture decoder rejected codec: '+('x'*int(sys.argv[1])));sys.stderr.flush();sys.exit(7)".into(), diagnostic_budget.saturating_mul(2).to_string()], true, policy).unwrap();
        pipeline.children[1].name = "fixture ffmpeg".into();
        pipeline.pipe_to_last(source).unwrap();
        let mut video = DeviceVideo {
            reader: BufReader::new(output),
            pipeline,
            revoked: host.require(&grant.id, ToolScope::Devices).unwrap(),
            policy: host.policy,
            first: None,
            _slot: Slot {
                host: Arc::downgrade(&host),
                grant: grant.id.clone(),
            },
        };
        let error = tokio::time::timeout(STDERR_SETTLE * 2, video.next_frame())
            .await
            .expect("A failed transcoder must not wait for its live upstream recorder")
            .unwrap_err()
            .to_string();
        assert!(error.contains("fixture ffmpeg"));
        assert!(error.contains("fixture decoder rejected codec"));
        assert!(!error.contains("no complete frame"));
        assert!(video.pipeline.children[1].diagnostic.lock().unwrap().len() <= diagnostic_budget);
        assert_eq!(
            // SAFETY: signal zero only observes this test-owned recorder.
            unsafe { libc::kill(i32::try_from(recorder).unwrap(), 0) },
            0
        );
        drop(video);
        wait_reaped(recorder, host.policy.timeout).await;
        assert!(host.video.lock().unwrap().is_empty());
    }

    #[cfg(unix)]
    async fn wait_reaped(pid: u32, timeout: Duration) {
        tokio::time::timeout(timeout, async {
            loop {
                // SAFETY: signal zero only observes a PID captured from our fixture.
                if unsafe { libc::kill(i32::try_from(pid).unwrap(), 0) } != 0 {
                    break;
                }
                tokio::task::yield_now().await;
            }
        })
        .await
        .expect("Owned recorder must be killed and reaped");
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn a_setup_error_stops_the_owned_recorder_group_and_drainers() {
        let policy = ProcessPolicy::default();
        let mut pipeline = Pipeline::default();
        let output = pipeline.spawn("/usr/bin/python3", &["-u".into(), "-c".into(), "import subprocess,time;child=subprocess.Popen(['sleep','3600']);print(child.pid,flush=True);time.sleep(3600)".into()], false, policy).unwrap();
        let recorder = pipeline.children[0].child.as_ref().unwrap().id().unwrap();
        let mut reader = BufReader::new(output);
        let mut proxy = String::new();
        tokio::time::timeout(policy.timeout, reader.read_line(&mut proxy))
            .await
            .unwrap()
            .unwrap();
        let result = (move || -> Result<(), ToolError> {
            let _output = pipeline.spawn("/bruhs-fixture-missing-transcoder", &[], true, policy)?;
            Ok(())
        })();
        assert!(result.is_err());
        wait_reaped(recorder, policy.timeout).await;
        tokio::time::timeout(policy.timeout, async {
            loop {
                let state = Command::new("ps")
                    .args(["-o", "stat=", "-p", proxy.trim()])
                    .output()
                    .await
                    .unwrap();
                let state = String::from_utf8_lossy(&state.stdout);
                if state.trim().is_empty() || state.trim_start().starts_with('Z') {
                    break;
                }
                tokio::task::yield_now().await;
            }
        })
        .await
        .expect("Only the fixture-owned descendant group must stop");
    }
}
