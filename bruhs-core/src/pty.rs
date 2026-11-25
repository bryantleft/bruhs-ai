//! Lean cross-platform pseudo-terminal (PTY) abstraction.
//!
//! On Unix (macOS and Linux), this uses `libc::openpty`, `TIOCSWINSZ`, and
//! `std::process::Command` directly with zero extra crate dependencies.
//! On Windows, it delegates to `portable-pty`'s ConPTY implementation and
//! maps errors to `std::io::Error` so callers never depend on `anyhow`.

use std::ffi::{OsStr, OsString};
use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct PtySize {
    pub rows: u16,
    pub cols: u16,
}

#[derive(Clone, Debug)]
pub(crate) enum EnvOp {
    Set(OsString, OsString),
    Remove(OsString),
}

#[derive(Clone, Debug)]
pub(crate) struct CommandBuilder {
    pub(crate) program: OsString,
    pub(crate) args: Vec<OsString>,
    pub(crate) cwd: Option<PathBuf>,
    pub(crate) env_ops: Vec<EnvOp>,
}

impl CommandBuilder {
    pub(crate) fn new(program: impl AsRef<OsStr>) -> Self {
        Self {
            program: program.as_ref().to_os_string(),
            args: Vec::new(),
            cwd: None,
            env_ops: Vec::new(),
        }
    }

    pub(crate) fn arg(&mut self, arg: impl AsRef<OsStr>) {
        self.args.push(arg.as_ref().to_os_string());
    }

    pub(crate) fn args<I, S>(&mut self, args: I)
    where
        I: IntoIterator<Item = S>,
        S: AsRef<OsStr>,
    {
        for arg in args {
            self.args.push(arg.as_ref().to_os_string());
        }
    }

    pub(crate) fn cwd(&mut self, dir: impl AsRef<Path>) {
        self.cwd = Some(dir.as_ref().to_path_buf());
    }

    pub(crate) fn env(&mut self, key: impl AsRef<OsStr>, value: impl AsRef<OsStr>) {
        self.env_ops.push(EnvOp::Set(
            key.as_ref().to_os_string(),
            value.as_ref().to_os_string(),
        ));
    }

    pub(crate) fn env_remove(&mut self, key: impl AsRef<OsStr>) {
        self.env_ops
            .push(EnvOp::Remove(key.as_ref().to_os_string()));
    }
}

pub(crate) struct PtyPair {
    pub master: MasterPty,
    pub slave: SlavePty,
}

#[cfg(unix)]
mod imp {
    use super::*;
    use std::fs::File;
    use std::os::fd::{AsRawFd, FromRawFd, OwnedFd};
    use std::os::unix::process::CommandExt as _;
    use std::process::{Child, Command, ExitStatus};
    use std::time::{Duration, Instant};

    pub(crate) struct MasterPty {
        fd: OwnedFd,
    }

    pub(crate) struct SlavePty {
        fd: OwnedFd,
    }

    pub(crate) struct PtyChild {
        child: Child,
        pid: libc::pid_t,
    }

    #[derive(Clone, Copy)]
    pub(crate) struct PtyKiller {
        pid: libc::pid_t,
    }

    fn set_cloexec(fd: libc::c_int) -> io::Result<()> {
        // SAFETY: `fd` is a freshly opened valid descriptor owned by this call.
        let rc = unsafe { libc::fcntl(fd, libc::F_SETFD, libc::FD_CLOEXEC) };
        if rc == -1 {
            return Err(io::Error::last_os_error());
        }
        Ok(())
    }

    pub(crate) fn openpty(size: PtySize) -> io::Result<PtyPair> {
        let mut master_fd: libc::c_int = -1;
        let mut slave_fd: libc::c_int = -1;
        let ws = libc::winsize {
            ws_row: size.rows,
            ws_col: size.cols,
            ws_xpixel: 0,
            ws_ypixel: 0,
        };
        // SAFETY: Valid mutable pointers to `master_fd`, `slave_fd`, and `ws`;
        // null name/termios pointers are permitted by `openpty(3)`.
        let rc = unsafe {
            libc::openpty(
                &mut master_fd,
                &mut slave_fd,
                std::ptr::null_mut(),
                std::ptr::null_mut(),
                std::ptr::from_ref(&ws).cast_mut(),
            )
        };
        if rc == -1 {
            return Err(io::Error::last_os_error());
        }
        // SAFETY: `openpty` succeeded and returned newly created valid fds.
        let master = unsafe { OwnedFd::from_raw_fd(master_fd) };
        // SAFETY: `openpty` succeeded and returned newly created valid fds.
        let slave = unsafe { OwnedFd::from_raw_fd(slave_fd) };
        set_cloexec(master.as_raw_fd())?;
        set_cloexec(slave.as_raw_fd())?;
        Ok(PtyPair {
            master: MasterPty { fd: master },
            slave: SlavePty { fd: slave },
        })
    }

    impl MasterPty {
        pub(crate) fn resize(&self, size: PtySize) -> io::Result<()> {
            let ws = libc::winsize {
                ws_row: size.rows,
                ws_col: size.cols,
                ws_xpixel: 0,
                ws_ypixel: 0,
            };
            // SAFETY: `self.fd` is a valid open PTY master fd and `ws` is valid.
            let rc = unsafe { libc::ioctl(self.fd.as_raw_fd(), libc::TIOCSWINSZ, &ws) };
            if rc == -1 {
                return Err(io::Error::last_os_error());
            }
            Ok(())
        }

        pub(crate) fn try_clone_reader(&self) -> io::Result<Box<dyn Read + Send>> {
            let dup = self.fd.try_clone()?;
            Ok(Box::new(File::from(dup)))
        }

        pub(crate) fn take_writer(&self) -> io::Result<Box<dyn Write + Send>> {
            let dup = self.fd.try_clone()?;
            Ok(Box::new(File::from(dup)))
        }
    }

    impl SlavePty {
        pub(crate) fn spawn_command(&self, spec: CommandBuilder) -> io::Result<PtyChild> {
            let mut command = Command::new(&spec.program);
            command.args(&spec.args);
            if let Some(cwd) = &spec.cwd {
                command.current_dir(cwd);
            }
            for op in &spec.env_ops {
                match op {
                    EnvOp::Set(k, v) => {
                        command.env(k, v);
                    }
                    EnvOp::Remove(k) => {
                        command.env_remove(k);
                    }
                }
            }
            let slave_raw = self.fd.as_raw_fd();
            // SAFETY: The closure runs in the forked child before `exec` and
            // invokes only async-signal-safe syscalls (`setsid`, `ioctl`,
            // `dup2`, `close`, `signal`). `slave_raw` remains open across
            // `fork` because `FD_CLOEXEC` only closes it at `exec`.
            unsafe {
                command.pre_exec(move || {
                    if libc::setsid() == -1 {
                        return Err(io::Error::last_os_error());
                    }
                    #[cfg_attr(
                        target_os = "linux",
                        expect(
                            clippy::useless_conversion,
                            reason = "TIOCSCTTY is already c_ulong on Linux"
                        )
                    )]
                    let req = libc::TIOCSCTTY.into();
                    if libc::ioctl(slave_raw, req, 0 as libc::c_int) == -1 {
                        return Err(io::Error::last_os_error());
                    }
                    for target in [0, 1, 2] {
                        if libc::dup2(slave_raw, target) == -1 {
                            return Err(io::Error::last_os_error());
                        }
                    }
                    if slave_raw > 2 {
                        let _ = libc::close(slave_raw);
                    }
                    // Restore default signal dispositions inherited as ignored.
                    libc::signal(libc::SIGPIPE, libc::SIG_DFL);
                    libc::signal(libc::SIGHUP, libc::SIG_DFL);
                    Ok(())
                });
            }
            let child = command.spawn()?;
            let pid = libc::pid_t::try_from(child.id())
                .map_err(|_| io::Error::other("child pid out of range"))?;
            Ok(PtyChild { child, pid })
        }
    }

    fn signal_process_group(pid: libc::pid_t, sig: libc::c_int) {
        if pid <= 0 {
            return;
        }
        // SAFETY: Sending a signal to a valid process group / PID returned by `spawn`.
        unsafe {
            let _ = libc::kill(-pid, sig);
            let _ = libc::kill(pid, sig);
        }
    }

    impl PtyKiller {
        pub(crate) fn kill(&mut self) -> io::Result<()> {
            signal_process_group(self.pid, libc::SIGHUP);
            signal_process_group(self.pid, libc::SIGKILL);
            Ok(())
        }
    }

    impl PtyChild {
        pub(crate) fn clone_killer(&self) -> PtyKiller {
            PtyKiller { pid: self.pid }
        }

        pub(crate) fn kill(&mut self) -> io::Result<()> {
            signal_process_group(self.pid, libc::SIGHUP);
            let deadline = Instant::now() + Duration::from_millis(50);
            while Instant::now() < deadline {
                if self.child.try_wait()?.is_some() {
                    return Ok(());
                }
                std::thread::sleep(Duration::from_millis(5));
            }
            signal_process_group(self.pid, libc::SIGKILL);
            Ok(())
        }

        pub(crate) fn wait(&mut self) -> io::Result<ExitStatus> {
            self.child.wait()
        }
    }
}

#[cfg(windows)]
mod imp {
    use super::*;

    pub(crate) struct MasterPty {
        inner: Box<dyn portable_pty::MasterPty + Send>,
    }

    pub(crate) struct SlavePty {
        inner: Box<dyn portable_pty::SlavePty + Send>,
    }

    pub(crate) struct PtyChild {
        inner: Box<dyn portable_pty::Child + Send + Sync>,
    }

    pub(crate) struct PtyKiller {
        inner: Box<dyn portable_pty::ChildKiller + Send + Sync>,
    }

    pub(crate) fn openpty(size: PtySize) -> io::Result<PtyPair> {
        let pair = portable_pty::native_pty_system()
            .openpty(portable_pty::PtySize {
                rows: size.rows,
                cols: size.cols,
                pixel_width: 0,
                pixel_height: 0,
            })
            .map_err(io::Error::other)?;
        Ok(PtyPair {
            master: MasterPty { inner: pair.master },
            slave: SlavePty { inner: pair.slave },
        })
    }

    impl MasterPty {
        pub(crate) fn resize(&self, size: PtySize) -> io::Result<()> {
            self.inner
                .resize(portable_pty::PtySize {
                    rows: size.rows,
                    cols: size.cols,
                    pixel_width: 0,
                    pixel_height: 0,
                })
                .map_err(io::Error::other)
        }

        pub(crate) fn try_clone_reader(&self) -> io::Result<Box<dyn Read + Send>> {
            self.inner.try_clone_reader().map_err(io::Error::other)
        }

        pub(crate) fn take_writer(&self) -> io::Result<Box<dyn Write + Send>> {
            self.inner.take_writer().map_err(io::Error::other)
        }
    }

    impl SlavePty {
        pub(crate) fn spawn_command(&self, spec: CommandBuilder) -> io::Result<PtyChild> {
            let mut cmd = portable_pty::CommandBuilder::new(&spec.program);
            cmd.args(&spec.args);
            if let Some(cwd) = &spec.cwd {
                cmd.cwd(cwd);
            }
            for op in &spec.env_ops {
                match op {
                    EnvOp::Set(k, v) => cmd.env(k, v),
                    EnvOp::Remove(k) => cmd.env_remove(k),
                }
            }
            let inner = self.inner.spawn_command(cmd).map_err(io::Error::other)?;
            Ok(PtyChild { inner })
        }
    }

    impl PtyKiller {
        pub(crate) fn kill(&mut self) -> io::Result<()> {
            self.inner.kill()
        }
    }

    pub(crate) struct WindowsExitStatus(portable_pty::ExitStatus);
    impl WindowsExitStatus {
        pub(crate) fn success(&self) -> bool {
            self.0.success()
        }
    }

    impl PtyChild {
        pub(crate) fn clone_killer(&self) -> PtyKiller {
            PtyKiller {
                inner: self.inner.clone_killer(),
            }
        }

        pub(crate) fn kill(&mut self) -> io::Result<()> {
            self.inner.kill()
        }

        pub(crate) fn wait(&mut self) -> io::Result<WindowsExitStatus> {
            self.inner.wait().map(WindowsExitStatus)
        }
    }
}

pub(crate) use imp::{openpty, MasterPty, PtyChild, SlavePty};
