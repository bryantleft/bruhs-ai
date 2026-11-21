//! User-local command that continues to point at the installed desktop across upgrades.
use std::{
    ffi::OsStr,
    io,
    path::{Path, PathBuf},
    process::Command,
};

pub enum Status {
    Available(PathBuf),
    Installed(PathBuf, bool),
    Occupied(PathBuf),
}
pub fn version() -> &'static str {
    let packaged = env!("BRUHS_RELEASE_VERSION");
    if packaged.is_empty() {
        env!("CARGO_PKG_VERSION")
    } else {
        packaged
    }
}

pub fn cli() -> Option<i32> {
    let mut args = std::env::args_os();
    let invoked = args.next()?;
    match args.next().as_deref() {
        Some(arg) if arg == OsStr::new("--version") || arg == OsStr::new("-V") => {
            println!("bruhs {}", version());
            Some(0)
        }
        Some(arg) if arg == OsStr::new("--help") || arg == OsStr::new("-h") => {
            println!("bruhs [--version | service <install|uninstall|status>]\n\nOpen the desktop workspace, or manage the bundled background service.");
            Some(0)
        }
        Some(arg) if arg == OsStr::new("service") => {
            let result = std::env::current_exe().and_then(|exe| {
                let host =
                    exe.with_file_name(format!("bruhs-server{}", std::env::consts::EXE_SUFFIX));
                Command::new(host).arg("service").args(args).status()
            });
            Some(exit_code(result))
        }
        None if cfg!(target_os = "macos")
            && Path::new(&invoked).file_name() == Some(OsStr::new("bruhs")) =>
        {
            let app = bruhs_core::updates::release::current_app_bundle()?;
            Some(exit_code(Command::new("/usr/bin/open").arg(app).status()))
        }
        _ => None,
    }
}
fn exit_code(result: io::Result<std::process::ExitStatus>) -> i32 {
    match result {
        Ok(status) => status.code().unwrap_or(1),
        Err(error) => {
            eprintln!("bruhs: {error}");
            1
        }
    }
}

pub fn configure(install: bool) -> io::Result<Status> {
    let exe = std::env::current_exe()?.canonicalize()?;
    let base = directories::BaseDirs::new()
        .ok_or_else(|| io::Error::other("Home directory unavailable"))?;
    let bin = if cfg!(windows) {
        base.data_local_dir().join("bruhs").join("bin")
    } else {
        base.home_dir().join(".local/bin")
    };
    let path: Vec<_> = std::env::var_os("PATH")
        .map(|p| {
            std::env::split_paths(&p)
                .filter(|p| p.is_absolute())
                .collect()
        })
        .unwrap_or_default();
    configure_at(&exe, &bin, &path, install)
}

fn configure_at(exe: &Path, bin: &Path, search: &[PathBuf], install: bool) -> io::Result<Status> {
    let name = if cfg!(windows) { "bruhs.cmd" } else { "bruhs" };
    let dest = bin.join(name);
    for dir in search
        .iter()
        .map(PathBuf::as_path)
        .chain(std::iter::once(bin))
    {
        let names: &[&str] = if cfg!(windows) {
            &["bruhs.com", "bruhs.exe", "bruhs.bat", "bruhs.cmd"]
        } else {
            &["bruhs"]
        };
        for name in names {
            let found = dir.join(name);
            match std::fs::symlink_metadata(&found) {
                Err(e) if e.kind() == io::ErrorKind::NotFound => continue,
                Err(e) => return Err(e),
                Ok(_) => {
                    let ours =
                        found.canonicalize().is_ok_and(|p| p == exe) || is_our_script(&found, exe);
                    return Ok(if ours {
                        Status::Installed(found, search.iter().any(|p| p == dir))
                    } else {
                        Status::Occupied(found)
                    });
                }
            }
        }
    }
    if !install {
        return Ok(Status::Available(dest));
    }
    std::fs::create_dir_all(bin)?;
    #[cfg(unix)]
    std::os::unix::fs::symlink(exe, &dest)?;
    #[cfg(windows)]
    {
        use std::io::Write;
        let mut file = tempfile::NamedTempFile::new_in(bin)?;
        file.write_all(script(exe)?.as_bytes())?;
        file.persist_noclobber(&dest)?;
    }
    Ok(Status::Installed(dest, search.iter().any(|p| p == bin)))
}
#[cfg(windows)]
fn script(exe: &Path) -> io::Result<String> {
    let text = exe.to_str().ok_or_else(|| {
        io::Error::other("Executable path cannot be represented in a command file")
    })?;
    Ok(format!(
        "@echo off\r\nsetlocal DisableDelayedExpansion\r\n\"{}\" %*\r\n",
        text.replace('%', "%%")
    ))
}
#[cfg(windows)]
fn is_our_script(file: &Path, exe: &Path) -> bool {
    file.extension()
        .is_some_and(|x| x.eq_ignore_ascii_case("cmd"))
        && script(exe).ok().is_some_and(|expected| {
            std::fs::metadata(file).is_ok_and(|m| m.len() == expected.len() as u64)
                && std::fs::read(file).is_ok_and(|bytes| bytes == expected.as_bytes())
        })
}
#[cfg(not(windows))]
fn is_our_script(_: &Path, _: &Path) -> bool {
    false
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn installation_is_idempotent_and_preserves_an_existing_command() {
        let tmp = tempfile::tempdir().unwrap();
        let exe = tmp.path().join("app");
        std::fs::write(&exe, b"fixture").unwrap();
        let exe = exe.canonicalize().unwrap();
        let bin = tmp.path().join("bin");
        let dirs = vec![bin.clone()];
        assert!(matches!(
            configure_at(&exe, &bin, &dirs, false).unwrap(),
            Status::Available(_)
        ));
        assert!(matches!(
            configure_at(&exe, &bin, &dirs, true).unwrap(),
            Status::Installed(_, true)
        ));
        let Status::Installed(command, true) = configure_at(&exe, &bin, &dirs, true).unwrap()
        else {
            panic!("Missing launcher")
        };
        std::fs::remove_file(&command).unwrap();
        std::fs::write(&command, b"another program").unwrap();
        assert!(matches!(
            configure_at(&exe, &bin, &dirs, true).unwrap(),
            Status::Occupied(_)
        ));
        assert_eq!(std::fs::read(command).unwrap(), b"another program");
    }
}
