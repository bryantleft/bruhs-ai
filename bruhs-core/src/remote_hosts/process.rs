use super::*;

// Reuse the CLI subprocess wrapper's stderr diagnostic budget.
const DIAGNOSTIC_BYTES: usize = 8 * 1024;

#[cfg(unix)]
struct Group(Option<i32>);
#[cfg(unix)]
impl Drop for Group {
    fn drop(&mut self) {
        if let Some(pid) = self.0.take() {
            // SAFETY: this positive group ID came from our child, created with a
            // fresh process group. No persisted PID or user session is targeted.
            unsafe {
                libc::kill(-pid, libc::SIGKILL);
            }
        }
    }
}

fn command(executable: &Path) -> Command {
    let mut command = Command::new(executable);
    command.env_clear();
    // Preserve only OpenSSH's ordinary OS/login inputs, not provider credentials.
    for key in [
        "PATH",
        "HOME",
        "USER",
        "LOGNAME",
        "SSH_AUTH_SOCK",
        "LANG",
        "LC_ALL",
        "TMPDIR",
        "SYSTEMROOT",
        "WINDIR",
        "USERPROFILE",
        "HOMEDRIVE",
        "HOMEPATH",
    ] {
        if let Some(value) = std::env::var_os(key) {
            command.env(key, value);
        }
    }
    command.stdin(Stdio::null()).kill_on_drop(true);
    command
}

pub(super) fn tunnel_args(profile: &RemoteHostProfile, control: &Path) -> Vec<String> {
    let mut args = vec![
        "-N".into(),
        "-T".into(),
        "-o".into(),
        "BatchMode=yes".into(),
        "-o".into(),
        "StrictHostKeyChecking=yes".into(),
        "-o".into(),
        "ExitOnForwardFailure=yes".into(),
        "-o".into(),
        "ControlMaster=yes".into(),
        "-o".into(),
        "ControlPersist=no".into(),
        "-o".into(),
        "ForkAfterAuthentication=no".into(),
        "-o".into(),
        "PermitLocalCommand=no".into(),
        "-o".into(),
        "ForwardAgent=no".into(),
        "-o".into(),
        "ForwardX11=no".into(),
        "-o".into(),
        format!("ConnectTimeout={}", profile.connect_timeout_seconds),
        "-S".into(),
        control.to_string_lossy().into(),
        "-L".into(),
        format!(
            "127.0.0.1:{}:127.0.0.1:{}",
            profile.local_port, profile.remote_port
        ),
        "-p".into(),
        profile.ssh_port.to_string(),
        "--".into(),
        profile.host.clone(),
    ];
    #[cfg(unix)]
    args.splice(
        0..0,
        [
            "-L".into(),
            format!(
                "{}:127.0.0.1:{}",
                control.with_file_name("api").display(),
                profile.remote_port
            ),
        ],
    );
    args
}

async fn bounded(
    mut input: impl tokio::io::AsyncRead + Unpin,
    limit: usize,
) -> std::io::Result<Vec<u8>> {
    let mut bytes = Vec::new();
    (&mut input)
        .take(u64::try_from(limit).unwrap_or(u64::MAX) + 1)
        .read_to_end(&mut bytes)
        .await?;
    if bytes.len() > limit {
        return Err(std::io::Error::other(
            "OpenSSH diagnostic output exceeded the CLI capture budget.",
        ));
    }
    Ok(bytes)
}

async fn output(executable: &Path, args: &[String]) -> std::io::Result<(bool, Vec<u8>, Vec<u8>)> {
    let mut prepared = command(executable);
    #[cfg(unix)]
    prepared.process_group(0);
    let mut child = prepared
        .args(args)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()?;
    #[cfg(unix)]
    let _group = Group(child.id().and_then(|id| i32::try_from(id).ok()));
    let stdout = child
        .stdout
        .take()
        .ok_or_else(|| std::io::Error::other("SSH stdout unavailable."))?;
    let stderr = child
        .stderr
        .take()
        .ok_or_else(|| std::io::Error::other("SSH stderr unavailable."))?;
    let (status, out, err) = tokio::try_join!(
        child.wait(),
        bounded(stdout, bruhs_models::mobile::REQUEST_BYTES),
        bounded(stderr, DIAGNOSTIC_BYTES)
    )?;
    Ok((status.success(), out, err))
}

impl RemoteHostManager {
    pub(super) async fn run_tunnel(
        &self,
        profile: &RemoteHostProfile,
        generation: Uuid,
        mut cancelled: watch::Receiver<bool>,
    ) -> Result<(), String> {
        let deadline = tokio::time::Instant::now()
            .checked_add(Duration::from_secs(u64::from(
                profile.connect_timeout_seconds,
            )))
            .ok_or_else(|| {
                "The SSH connection budget cannot be represented on this host.".to_owned()
            })?;
        if *cancelled.borrow() {
            return Ok(());
        }
        let configuration_args = [
            "-G".into(),
            "-p".into(),
            profile.ssh_port.to_string(),
            "--".into(),
            profile.host.clone(),
        ];
        let configuration = tokio::select! {
            _=cancelled.changed()=>return Ok(()),
            result=tokio::time::timeout_at(deadline,output(&self.0.executable,&configuration_args))=>result.map_err(|_|"OpenSSH configuration did not finish within the chosen connection budget.".to_owned())?.map_err(|e|format!("Cannot inspect OpenSSH: {e}. Install OpenSSH and check its PATH."))?,
        };
        if !configuration.0 {
            return Err(format!(
                "OpenSSH could not resolve this host: {}",
                String::from_utf8_lossy(&configuration.2)
            ));
        }
        let text = String::from_utf8(configuration.1)
            .map_err(|_| "OpenSSH returned unreadable configuration.".to_owned())?;
        if text.lines().any(|line| {
            line.split_whitespace().next().is_some_and(|key| {
                ["localforward", "remoteforward", "dynamicforward"].contains(&key)
            })
        }) {
            return Err("This SSH alias defines additional forwards. Create an alias without forwarding directives so Bruhs can own only the reviewed loopback tunnel.".into());
        }
        if *cancelled.borrow() {
            return Ok(());
        }
        let directory = tempfile::Builder::new()
            .prefix("bruhs-ssh-")
            .tempdir()
            .map_err(|e| format!("Cannot create a private SSH control directory: {e}"))?;
        let control = directory.path().join("control");
        let check_args = [
            "-S".into(),
            control.to_string_lossy().into(),
            "-O".into(),
            "check".into(),
            "--".into(),
            profile.host.clone(),
        ];
        let mut ssh = command(&self.0.executable);
        ssh.args(tunnel_args(profile, &control))
            .stdout(Stdio::null())
            .stderr(Stdio::piped());
        #[cfg(unix)]
        ssh.process_group(0);
        let mut child = ssh
            .spawn()
            .map_err(|e| format!("Cannot start OpenSSH: {e}. Install it and check its PATH."))?;
        #[cfg(unix)]
        let _group = Group(child.id().and_then(|id| i32::try_from(id).ok()));
        let mut stderr = child
            .stderr
            .take()
            .ok_or_else(|| "SSH diagnostics are unavailable.".to_owned())?;
        let tail = Arc::new(std::sync::Mutex::new(Vec::new()));
        let captured = tail.clone();
        let mut drain = tokio::spawn(async move {
            let mut bytes = [0u8; 1024];
            loop {
                match stderr.read(&mut bytes).await {
                    Ok(0) | Err(_) => break,
                    Ok(n) => {
                        if let Ok(mut tail) = captured.lock() {
                            tail.extend_from_slice(&bytes[..n]);
                            let remove = tail.len().saturating_sub(DIAGNOSTIC_BYTES);
                            if remove > 0 {
                                tail.drain(..remove);
                            }
                        }
                    }
                }
            }
        });
        let checked=async {
            loop {
                if *cancelled.borrow(){return Ok(());}
                if let Some(status)=child.try_wait().map_err(|e|e.to_string())? {
                    return Err(format!("SSH exited before the tunnel was ready ({status}). Verify this host's key and noninteractive login in a terminal, then retry."));
                }
                let check=output(&self.0.executable,&check_args);
                let result=tokio::select! {_=cancelled.changed()=>return Ok(()),result=tokio::time::timeout_at(deadline,check)=>result.map_err(|_|"SSH did not authenticate and acknowledge its private control socket within the chosen connection budget.".to_owned())?.map_err(|e|e.to_string())?};
                if result.0 {self.connected(profile.id,generation,cfg!(unix).then(||control.with_file_name("api"))).await.map_err(|e|e.to_string())?;break;}
                // Reuse the native mobile connection-status cadence; probes are
                // local control-socket commands, not new network connections.
                tokio::select! {_=cancelled.changed()=>return Ok(()),_ = tokio::time::sleep_until(std::cmp::min(deadline,tokio::time::Instant::now()+Duration::from_millis(bruhs_models::mobile::REFRESH_MS)))=>{}}
                if tokio::time::Instant::now()>=deadline{return Err("SSH authentication/control readiness exceeded the chosen connection budget. Check known_hosts and your existing SSH login.".into());}
            }
            tokio::select! {
                _=cancelled.changed()=>Ok(()),
                status=child.wait()=>{
                    let status=status.map_err(|e|e.to_string())?;
                    Err(format!("The owned SSH tunnel ended ({status}). Connect again after inspecting the host."))
                }
            }
        }.await;
        let _ = child.kill().await;
        let _ = child.wait().await;
        #[cfg(unix)]
        drop(_group);
        // Reuse the shared CLI stderr-settle deadline after killing the group.
        if tokio::time::timeout(Duration::from_secs(1), &mut drain)
            .await
            .is_err()
        {
            drain.abort();
        }
        let diagnostic = tail
            .lock()
            .map(|bytes| String::from_utf8_lossy(&bytes).into_owned())
            .unwrap_or_default();
        drop(directory);
        match checked {
            Err(message) if !diagnostic.is_empty() => Err(format!("{message}\n{diagnostic}")),
            result => result,
        }
    }
}
