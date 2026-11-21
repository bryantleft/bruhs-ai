//! Stopping the background service for a restore and starting it again
//! after (D15, D16, ruling F22), in a way the detached applier survives.
//!
//! The job stays loaded throughout. On macOS a restore sends the service's
//! process SIGTERM: it shuts down cleanly and exits 0, which
//! `KeepAlive.SuccessfulExit=false` doesn't restart, and the applier starts
//! it again with `launchctl kickstart` (`bootstrap` refuses a job that is
//! still loaded). On Linux it is `systemctl --user stop` and `start`, never
//! `disable --now`. Nothing here runs unless the service's definition
//! exists under this process's `HOME`, so a scratch home never stops the
//! owner's real service (the gui domain is per user, not per home).

use super::{
    definition_path, gui_domain, launchd_pid, parse_launchctl_print, parse_systemctl_show, ran,
    real_runner, Asker, Clock, JobState, Launchd, QueryOutput, ServiceError, Settle, Systemd,
    LABEL, REAL_CLOCK, UNIT,
};

/// How long a stopping service gets to shut down: its own grace for open
/// connections, and closing the bruhs' browsers.
const STOP_SETTLE: Settle = Settle {
    deadline: std::time::Duration::from_secs(30),
    interval: std::time::Duration::from_millis(200),
};

/// The service's job as a restore sees it; launchd or systemd in use, a
/// fake in tests.
pub(crate) trait Job {
    /// `None` when the job isn't loaded (or, under systemd, not active);
    /// otherwise its process, if it has one. A state that can't be read is
    /// an error.
    fn process(&self) -> Result<Option<Option<u32>>, ServiceError>;
    /// Stops the job's process, `pid` when it has one, and confirms it is
    /// gone. The job stays loaded.
    fn stop(&self, pid: Option<u32>) -> Result<(), ServiceError>;
    /// Starts the loaded job again.
    fn start(&self) -> Result<(), ServiceError>;
}

/// Stops the installed service before a restore. `Ok(true)` when it was
/// loaded, so the applier starts it again after; `Ok(false)` when it isn't
/// installed under this `HOME` or isn't loaded. When `asker` is the
/// service's own process, nothing is signalled: that process stops by
/// shutting down after it answers (ruling F22 a).
pub(crate) fn stop_for_restore(asker: Asker) -> Result<bool, ServiceError> {
    let installed = definition_path()?.is_file();
    if !installed {
        return Ok(false);
    }
    let own = match asker {
        Asker::Server { pid } => Some(pid),
        Asker::Cli { .. } => None,
    };
    stop_job(installed, real_job()?.as_ref(), own)
}

/// Starts the installed service again after a restore, keeping its
/// definition as it is. Does nothing unless it is installed under this
/// `HOME`.
pub(crate) fn start_after_restore() -> Result<(), ServiceError> {
    let installed = definition_path()?.is_file();
    if !installed {
        return Ok(());
    }
    start_job(installed, real_job()?.as_ref())
}

fn stop_job(installed: bool, job: &dyn Job, own: Option<u32>) -> Result<bool, ServiceError> {
    if !installed {
        return Ok(false);
    }
    match job.process()? {
        None => Ok(false),
        Some(Some(pid)) if Some(pid) == own => Ok(true),
        Some(pid) => job.stop(pid).map(|()| true),
    }
}

fn start_job(installed: bool, job: &dyn Job) -> Result<(), ServiceError> {
    if installed {
        job.start()
    } else {
        Ok(())
    }
}

fn real_job() -> Result<Box<dyn Job>, ServiceError> {
    if cfg!(target_os = "macos") {
        Ok(Box::new(Launchd {
            domain: gui_domain()?,
            label: LABEL,
            runner: &real_runner,
            clock: &REAL_CLOCK,
            settle: STOP_SETTLE,
        }))
    } else if cfg!(target_os = "linux") {
        Ok(Box::new(Systemd {
            unit: UNIT,
            runner: &real_runner,
            clock: &REAL_CLOCK,
            settle: STOP_SETTLE,
        }))
    } else {
        Err(ServiceError::Unsupported(
            "Background service restores support macOS and Linux.".into(),
        ))
    }
}

/// Asks `process` until the job has no process, or fails at the deadline.
fn wait_gone(job: &dyn Job, clock: &dyn Clock, settle: Settle) -> Result<(), ServiceError> {
    let started = clock.now();
    loop {
        if !matches!(job.process()?, Some(Some(_))) {
            return Ok(());
        }
        if clock.now().saturating_duration_since(started) >= settle.deadline {
            return Err(ServiceError::Failed(
                "The background service is still running after the request to stop it.".into(),
            ));
        }
        clock.sleep(settle.interval);
    }
}

fn read_state(
    output: Result<QueryOutput, String>,
    parse: impl Fn(&QueryOutput) -> JobState,
    pid: impl Fn(&QueryOutput) -> Option<u32>,
) -> Result<Option<Option<u32>>, ServiceError> {
    let output = output.map_err(ServiceError::Failed)?;
    match parse(&output) {
        JobState::Running => Ok(Some(pid(&output))),
        JobState::NotLoaded | JobState::Inactive => Ok(None),
        JobState::Indeterminate(why) => Err(ServiceError::Failed(format!(
            "Could not read the background service's state ({why})."
        ))),
    }
}

impl Job for Launchd<'_> {
    fn process(&self) -> Result<Option<Option<u32>>, ServiceError> {
        let target = format!("{}/{}", self.domain, self.label);
        read_state(
            (self.runner)("launchctl", &["print", &target]),
            |output| parse_launchctl_print(output, self.label),
            |output| launchd_pid(&output.stdout),
        )
    }

    /// Signals the job's process. A loaded job with none has nothing to
    /// stop, and launchd may still start it during the restore (its
    /// `KeepAlive`, after a throttle): that start meets the start check,
    /// which waits for the restore before opening anything (ruling F20 b,
    /// final review C-M5).
    fn stop(&self, pid: Option<u32>) -> Result<(), ServiceError> {
        if pid.is_none() {
            return Ok(());
        }
        let target = format!("{}/{}", self.domain, self.label);
        ran((self.runner)("launchctl", &["kill", "SIGTERM", &target]))?;
        wait_gone(self, self.clock, self.settle)
    }

    fn start(&self) -> Result<(), ServiceError> {
        let target = format!("{}/{}", self.domain, self.label);
        ran((self.runner)("launchctl", &["kickstart", &target])).map(drop)
    }
}

impl Job for Systemd<'_> {
    fn process(&self) -> Result<Option<Option<u32>>, ServiceError> {
        read_state(
            (self.runner)(
                "systemctl",
                &[
                    "--user",
                    "show",
                    self.unit,
                    "--property=LoadState,ActiveState,MainPID",
                ],
            ),
            parse_systemctl_show,
            |output| {
                output
                    .stdout
                    .lines()
                    .find_map(|line| line.strip_prefix("MainPID="))
                    .and_then(|pid| pid.trim().parse::<u32>().ok())
                    .filter(|pid| *pid != 0)
            },
        )
    }

    /// `stop` also cancels a restart systemd has queued, so the job stays
    /// down while the restore runs.
    fn stop(&self, _pid: Option<u32>) -> Result<(), ServiceError> {
        ran((self.runner)("systemctl", &["--user", "stop", self.unit]))?;
        wait_gone(self, self.clock, self.settle)
    }

    fn start(&self) -> Result<(), ServiceError> {
        ran((self.runner)("systemctl", &["--user", "start", self.unit])).map(drop)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::{Cell, RefCell};

    /// A job that answers as told and counts what it was asked.
    struct FakeJob {
        loaded: Result<Option<Option<u32>>, ()>,
        stops: Cell<u32>,
        starts: Cell<u32>,
    }

    impl FakeJob {
        fn new(loaded: Result<Option<Option<u32>>, ()>) -> Self {
            Self {
                loaded,
                stops: Cell::new(0),
                starts: Cell::new(0),
            }
        }
    }

    impl Job for FakeJob {
        fn process(&self) -> Result<Option<Option<u32>>, ServiceError> {
            self.loaded
                .map_err(|()| ServiceError::Failed("unknown".into()))
        }
        fn stop(&self, _pid: Option<u32>) -> Result<(), ServiceError> {
            self.stops.set(self.stops.get() + 1);
            Ok(())
        }
        fn start(&self) -> Result<(), ServiceError> {
            self.starts.set(self.starts.get() + 1);
            Ok(())
        }
    }

    #[test]
    fn stopping_for_a_restore_stops_only_a_loaded_service() {
        let own = Some(7);
        // Installed and running: stopped once.
        let job = FakeJob::new(Ok(Some(Some(42))));
        assert!(stop_job(true, &job, own).unwrap());
        assert_eq!(job.stops.get(), 1);
        // Loaded with no process: still asked to stop, and restarted after.
        // systemd's stop cancels a restart it queued; launchd has nothing
        // to signal (see `a_restore_keeps_the_launchd_job_loaded`).
        let job = FakeJob::new(Ok(Some(None)));
        assert!(stop_job(true, &job, own).unwrap());
        assert_eq!(job.stops.get(), 1);
        // Not installed under this home: nothing asked, nothing to restart.
        let job = FakeJob::new(Ok(Some(Some(42))));
        assert!(!stop_job(false, &job, own).unwrap());
        assert_eq!(job.stops.get(), 0);
        // Installed but not loaded.
        let job = FakeJob::new(Ok(None));
        assert!(!stop_job(true, &job, own).unwrap());
        assert_eq!(job.stops.get(), 0);
        // This process is the service: it stops by exiting, never by a
        // signal from itself.
        let job = FakeJob::new(Ok(Some(Some(7))));
        assert!(stop_job(true, &job, own).unwrap());
        assert_eq!(job.stops.get(), 0);
        // A state that can't be read is an error, never "not running".
        let job = FakeJob::new(Err(()));
        assert!(stop_job(true, &job, own).is_err());
        assert_eq!(job.stops.get(), 0);
    }

    #[test]
    fn starting_after_a_restore_starts_the_installed_job() {
        let job = FakeJob::new(Ok(None));
        start_job(true, &job).unwrap();
        assert_eq!(job.starts.get(), 1);
        let job = FakeJob::new(Ok(None));
        start_job(false, &job).unwrap();
        assert_eq!(job.starts.get(), 0);
    }

    /// A clock that moves only when asked to sleep.
    struct StepClock {
        base: std::time::Instant,
        elapsed: Cell<std::time::Duration>,
    }
    impl Clock for StepClock {
        fn now(&self) -> std::time::Instant {
            self.base + self.elapsed.get()
        }
        fn sleep(&self, pause: std::time::Duration) {
            self.elapsed.set(self.elapsed.get() + pause);
        }
    }
    fn clock() -> StepClock {
        StepClock {
            base: std::time::Instant::now(),
            elapsed: Cell::new(std::time::Duration::ZERO),
        }
    }

    fn output(code: i32, stdout: &str, stderr: &str) -> QueryOutput {
        QueryOutput {
            code: Some(code),
            stdout: stdout.into(),
            stderr: stderr.into(),
        }
    }

    #[test]
    fn a_restore_keeps_the_launchd_job_loaded() {
        let calls: &'static RefCell<Vec<String>> = Box::leak(Box::default());
        let killed: &'static Cell<bool> = Box::leak(Box::default());
        let runner = move |program: &str, args: &[&str]| {
            calls
                .borrow_mut()
                .push(format!("{program} {}", args.join(" ")));
            match args.first().copied() {
                Some("print") if killed.get() => Ok(output(0, "state = not running\n", "")),
                Some("print") => Ok(output(0, "state = running\n\tpid = 4242\n", "")),
                Some("kill") => {
                    killed.set(true);
                    Ok(output(0, "", ""))
                }
                Some("kickstart") => Ok(output(0, "", "")),
                _ => Ok(output(5, "", "unexpected")),
            }
        };
        let clock = clock();
        let job = Launchd {
            domain: "gui/502".into(),
            label: "dev.bruhs.server",
            runner: &runner,
            clock: &clock,
            settle: STOP_SETTLE,
        };
        assert_eq!(job.process().unwrap(), Some(Some(4242)));
        job.stop(Some(4242)).unwrap();
        assert_eq!(job.process().unwrap(), Some(None));
        job.start().unwrap();
        let calls = calls.borrow().clone();
        assert!(calls.contains(&"launchctl kill SIGTERM gui/502/dev.bruhs.server".to_owned()));
        assert!(calls.contains(&"launchctl kickstart gui/502/dev.bruhs.server".to_owned()));
        assert!(
            !calls
                .iter()
                .any(|call| call.contains("bootout") || call.contains("bootstrap")),
            "{calls:?}"
        );
        // A loaded job with no process has nothing to signal, and nothing
        // is asked of launchd: a start it makes meanwhile meets the start
        // check, which waits for the restore (final review C-M5).
        let asked: &'static Cell<u32> = Box::leak(Box::default());
        let quiet = |_: &str, args: &[&str]| {
            asked.set(asked.get() + 1);
            match args.first().copied() {
                Some("kill") => Ok(output(3, "", "No such process")),
                _ => Ok(output(0, "state = not running\n", "")),
            }
        };
        let job = Launchd {
            domain: "gui/502".into(),
            label: "dev.bruhs.server",
            runner: &quiet,
            clock: &clock,
            settle: STOP_SETTLE,
        };
        job.stop(None).unwrap();
        assert_eq!(asked.get(), 0);
        // A process that never leaves is an error, not a stop.
        let stuck = |_: &str, args: &[&str]| match args.first().copied() {
            Some("print") => Ok(output(0, "\tpid = 4242\n", "")),
            _ => Ok(output(0, "", "")),
        };
        let job = Launchd {
            domain: "gui/502".into(),
            label: "dev.bruhs.server",
            runner: &stuck,
            clock: &clock,
            settle: STOP_SETTLE,
        };
        assert!(job.stop(Some(4242)).is_err());
    }

    #[test]
    fn a_restore_keeps_the_systemd_unit_enabled() {
        let calls: &'static RefCell<Vec<String>> = Box::leak(Box::default());
        let stopped: &'static Cell<bool> = Box::leak(Box::default());
        let runner = move |program: &str, args: &[&str]| {
            calls
                .borrow_mut()
                .push(format!("{program} {}", args.join(" ")));
            match args.get(1).copied() {
                Some("show") if stopped.get() => Ok(output(
                    0,
                    "LoadState=loaded\nActiveState=inactive\nMainPID=0\n",
                    "",
                )),
                Some("show") => Ok(output(
                    0,
                    "LoadState=loaded\nActiveState=active\nMainPID=4242\n",
                    "",
                )),
                Some("stop") => {
                    stopped.set(true);
                    Ok(output(0, "", ""))
                }
                Some("start") => Ok(output(0, "", "")),
                _ => Ok(output(5, "", "unexpected")),
            }
        };
        let clock = clock();
        let job = Systemd {
            unit: "bruhs-server.service",
            runner: &runner,
            clock: &clock,
            settle: STOP_SETTLE,
        };
        assert_eq!(job.process().unwrap(), Some(Some(4242)));
        job.stop(Some(4242)).unwrap();
        assert_eq!(job.process().unwrap(), None);
        job.start().unwrap();
        let calls = calls.borrow().clone();
        assert!(calls.contains(&"systemctl --user stop bruhs-server.service".to_owned()));
        assert!(calls.contains(&"systemctl --user start bruhs-server.service".to_owned()));
        assert!(
            !calls
                .iter()
                .any(|call| call.contains("disable") || call.contains("enable")),
            "{calls:?}"
        );
    }
}
