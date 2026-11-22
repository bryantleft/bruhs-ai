use super::*;
use futures_util::StreamExt;
use sha2::{Digest, Sha256};
use std::sync::Arc;
use std::{fs, process::Stdio};
use tempfile::TempDir;
use tokio::io::AsyncWriteExt;

pub(super) const MAX_ARCHIVE: u64 = 1024 * 1024 * 1024;

pub struct Prepared {
    pub command: PathBuf,
    pub args: Vec<String>,
    pub environment: BTreeMap<String, String>,
}

pub async fn prepare(agent: &Agent) -> io::Result<Prepared> {
    prepare_at(agent, &super::super::directory()?.join("registry/tools")).await
}

async fn prepare_at(agent: &Agent, root: &Path) -> io::Result<Prepared> {
    if !identifier(&agent.id) {
        return Err(io::Error::other("Invalid registry agent identifier."));
    }
    let digest = Sha256::digest(serde_json::to_vec(&(
        &agent.id,
        &agent.version,
        &agent.distribution,
        super::native_target(),
    ))?);
    let dir = root.join(&agent.id).join(format!("{digest:x}"));
    let locked = dir.clone();
    // Package entry points may contain their absolute install directory. Keep the
    // installation there and atomically publish its launcher after setup succeeds.
    let _lock = tokio::task::spawn_blocking(move || super::super::lock_config(&locked))
        .await
        .map_err(io::Error::other)??;
    let dir = dir.canonicalize()?;
    let marker = dir.join("launcher.json");
    let launch = match &agent.distribution {
        Distribution::Binary { launch, .. } | Distribution::Package { launch, .. } => launch,
    };
    let result = |command: PathBuf| Prepared {
        command,
        args: launch.args.clone(),
        environment: launch.env.clone(),
    };
    let saved = fs::read(&marker)
        .ok()
        .and_then(|bytes| serde_json::from_slice::<PathBuf>(&bytes).ok());
    if let Some(saved) = saved {
        if saved.is_relative()
            && saved
                .components()
                .all(|c| matches!(c, Component::Normal(_)))
        {
            if let Ok(path) = local_executable(&dir, &saved) {
                return Ok(result(path));
            }
        }
    }
    let stage = Arc::new(
        tempfile::Builder::new()
            .prefix("install-")
            .tempdir_in(&dir)?,
    );
    let command = match &agent.distribution {
        Distribution::Binary {
            archive,
            cmd,
            sha256,
            ..
        } => install_binary(stage.clone(), archive, cmd, sha256.as_deref()).await?,
        Distribution::Package {
            manager,
            name,
            version,
            ..
        } => install_package(agent, *manager, name, version, &stage).await?,
    };
    let command = local_executable(stage.path(), &command)?;
    let stage =
        Arc::try_unwrap(stage).map_err(|_| io::Error::other("Installation is still in use."))?;
    let mut tmp = tempfile::NamedTempFile::new_in(&dir)?;
    serde_json::to_writer(
        &mut tmp,
        command.strip_prefix(&dir).map_err(io::Error::other)?,
    )?;
    tmp.as_file().sync_all()?;
    tmp.persist(&marker)?;
    let _ = stage.keep();
    Ok(result(command))
}

fn local_executable(root: &Path, command: &Path) -> io::Result<PathBuf> {
    let path = root.join(command);
    let real = path.canonicalize()?;
    if !real.starts_with(root.canonicalize()?) || !real.is_file() {
        return Err(io::Error::other(
            "Registry launcher is outside its installation.",
        ));
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        if fs::metadata(&real)?.permissions().mode() & 0o111 == 0 {
            return Err(io::Error::other("Registry launcher is not executable."));
        }
    }
    Ok(path)
}

async fn install_package(
    agent: &Agent,
    manager: Manager,
    name: &str,
    version: &str,
    stage: &TempDir,
) -> io::Result<PathBuf> {
    let exe = manager.executable().ok_or_else(|| {
        io::Error::other(format!(
            "{} is unavailable on this computer.",
            manager.tool()
        ))
    })?;
    let dir = stage.path();
    let mut cmd = tokio::process::Command::new(exe);
    cmd.current_dir(dir)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .kill_on_drop(true);
    crate::connections::protect_command(&mut cmd);
    let bins = match manager {
        Manager::Npm => {
            cmd.args(["install", "--no-audit", "--no-fund", "--prefix"])
                .arg(dir)
                .arg("--")
                .arg(format!("{name}@{version}"));
            dir.join("node_modules/.bin")
        }
        Manager::Uv => {
            let bins = dir.join("bin");
            cmd.env("UV_TOOL_DIR", dir.join("tools"))
                .env("UV_TOOL_BIN_DIR", &bins)
                .args(["tool", "install", "--"])
                .arg(format!("{name}=={version}"));
            bins
        }
    };
    #[cfg(unix)]
    cmd.process_group(0);
    let mut child = cmd.spawn()?;
    #[cfg(unix)]
    let mut group = crate::agent::ProcessGroup::of(&child);
    let status = tokio::time::timeout(Duration::from_secs(1200), child.wait())
        .await
        .map_err(|_| io::Error::other("The package installation timed out."))??;
    #[cfg(unix)]
    group.release();
    if !status.success() {
        return Err(io::Error::other(format!(
            "{} could not install {name} {version} ({status}).",
            manager.tool()
        )));
    }
    let plain = name.rsplit('/').next().unwrap_or(name);
    let mut candidates = vec![
        agent.id.clone(),
        plain.into(),
        plain.trim_end_matches("-acp").into(),
    ];
    if matches!(manager, Manager::Npm) {
        // Only consider bins declared by the chosen package, never its dependencies.
        let manifest: serde_json::Value = serde_json::from_slice(&fs::read(
            dir.join("node_modules").join(name).join("package.json"),
        )?)?;
        let declared: Vec<&str> = match &manifest["bin"] {
            serde_json::Value::String(_) => vec![plain],
            serde_json::Value::Object(b) => b
                .keys()
                .map(String::as_str)
                .filter(|k| identifier(k))
                .collect(),
            _ => vec![],
        };
        candidates.retain(|c| declared.contains(&c.as_str()));
        if let [only] = declared[..] {
            candidates.push(only.into())
        }
    } else {
        let entries = fs::read_dir(&bins)?.collect::<io::Result<Vec<_>>>()?;
        if let [entry] = entries.as_slice() {
            if let Some(n) = entry.file_name().to_str() {
                candidates.push(n.into())
            }
        }
    }
    for n in candidates {
        let launcher = bins.join(&n);
        #[cfg(windows)]
        let launcher = if launcher.is_file() {
            launcher
        } else {
            launcher.with_extension(if matches!(manager, Manager::Npm) {
                "cmd"
            } else {
                "exe"
            })
        };
        if launcher.is_file() {
            return Ok(launcher);
        }
    }
    Err(io::Error::other(
        "The package was installed but its agent executable could not be identified.",
    ))
}

async fn install_binary(
    stage: Arc<TempDir>,
    archive: &str,
    cmd: &Path,
    checksum: Option<&str>,
) -> io::Result<PathBuf> {
    let dir = stage.path();
    let file = tempfile::NamedTempFile::new_in(dir)?;
    let response = client(Duration::from_secs(1200))?
        .get(archive)
        .send()
        .await
        .map_err(io::Error::other)?
        .error_for_status()
        .map_err(io::Error::other)?;
    store_download(response, file.reopen()?, checksum).await?;
    let path = super::https(archive)
        .ok_or_else(|| io::Error::other("Invalid registry archive URL."))?
        .path()
        .to_lowercase();
    let dest = dir.join("files");
    fs::create_dir(&dest)?;
    let command = cmd.to_owned();
    tokio::task::spawn_blocking(move || {
        let _stage = stage;
        super::archive::unpack(file.as_file(), &path, &dest, &command)?;
        let launcher = dest.join(command);
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let resolved = launcher.canonicalize()?;
            if !resolved.starts_with(dest.canonicalize()?) || !resolved.is_file() {
                return Err(io::Error::other("Invalid registry launcher."));
            }
            let mut mode = fs::metadata(&resolved)?.permissions();
            mode.set_mode(mode.mode() | 0o700);
            fs::set_permissions(resolved, mode)?;
        }
        Ok(launcher)
    })
    .await
    .map_err(io::Error::other)?
}

async fn store_download(
    response: reqwest::Response,
    file: fs::File,
    checksum: Option<&str>,
) -> io::Result<()> {
    let mut output = tokio::fs::File::from_std(file);
    let mut received = 0;
    let mut hash = Sha256::new();
    let mut stream = response.bytes_stream();
    while let Some(chunk) = stream.next().await {
        let chunk = chunk.map_err(io::Error::other)?;
        received += chunk.len() as u64;
        if received > MAX_ARCHIVE {
            return Err(io::Error::other("Registry download exceeds 1 GiB."));
        }
        hash.update(&chunk);
        output.write_all(&chunk).await?;
    }
    output.flush().await?;
    drop(output);
    if checksum
        .is_some_and(|expected| !format!("{:x}", hash.finalize()).eq_ignore_ascii_case(expected))
    {
        return Err(io::Error::other(
            "The downloaded agent did not match its registry checksum.",
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[tokio::test]
    async fn downloaded_bytes_must_match_the_advertised_checksum() {
        let response = || reqwest::Response::from(hyper::Response::new(reqwest::Body::from("abc")));
        let good = format!("{:x}", Sha256::digest(b"abc"));
        store_download(response(), tempfile::tempfile().unwrap(), Some(&good))
            .await
            .unwrap();
        let err = store_download(
            response(),
            tempfile::tempfile().unwrap(),
            Some(&"0".repeat(64)),
        )
        .await
        .unwrap_err();
        assert!(err.to_string().contains("checksum"));
    }
}
