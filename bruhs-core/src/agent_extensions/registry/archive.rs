use std::{
    fs,
    io::{self, Read},
    path::{Component, Path, PathBuf},
};
const MAX_UNPACKED: u64 = 4 * super::install::MAX_ARCHIVE;
pub(super) fn unpack(file: &fs::File, format: &str, root: &Path, command: &Path) -> io::Result<()> {
    let mut remaining = MAX_UNPACKED;
    let mut links = Vec::new();
    if format.ends_with(".zip") {
        let mut zip = zip::ZipArchive::new(file)?;
        if zip.len() > 100_000 {
            return Err(io::Error::other("Too many registry archive entries."));
        }
        for i in 0..zip.len() {
            let mut e = zip.by_index(i)?;
            let Some(name) = e.enclosed_name() else {
                return Err(io::Error::other("Invalid archive entry."));
            };
            if e.is_symlink() {
                let mut target = String::new();
                e.take(4097).read_to_string(&mut target)?;
                if target.len() > 4096 {
                    return Err(io::Error::other("Registry link target is too long."));
                }
                links.push((name, PathBuf::from(target), false));
                continue;
            }
            if e.is_dir() {
                fs::create_dir_all(root.join(name))?
            } else {
                let mode = e.unix_mode().unwrap_or(0o644);
                write_entry(root, &name, &mut e, mode, &mut remaining)?
            }
        }
    } else if format.ends_with(".tar.gz")
        || format.ends_with(".tgz")
        || format.ends_with(".tar.bz2")
        || format.ends_with(".tbz2")
    {
        let stream: Box<dyn Read> = if format.ends_with("bz2") {
            Box::new(bzip2::read::BzDecoder::new(file))
        } else {
            Box::new(flate2::read::GzDecoder::new(file))
        };
        let mut archive = tar::Archive::new(stream);
        for (i, entry) in archive.entries()?.enumerate() {
            if i >= 100_000 {
                return Err(io::Error::other("Too many registry archive entries."));
            }
            let mut e = entry?;
            let name = e.path()?.into_owned();
            if name
                .components()
                .any(|c| !matches!(c, Component::CurDir | Component::Normal(_)))
            {
                return Err(io::Error::other("Invalid archive path."));
            }
            let kind = e.header().entry_type();
            if kind.is_dir() {
                fs::create_dir_all(root.join(name))?
            } else if kind.is_file() {
                let mode = e.header().mode()?;
                write_entry(root, &name, &mut e, mode, &mut remaining)?
            } else if kind.is_symlink() || kind.is_hard_link() {
                let target = e
                    .link_name()?
                    .ok_or_else(|| io::Error::other("Missing registry link target."))?;
                links.push((name, target.into_owned(), kind.is_hard_link()));
            } else {
                return Err(io::Error::other(
                    "Registry tar contains an unsupported entry.",
                ));
            }
        }
    } else {
        write_entry(root, command, file, 0o755, &mut remaining)?
    }
    // Defer links so extracting regular files can never follow an archive link.
    for (name, target, _) in &links {
        if name
            .components()
            .any(|c| !matches!(c, Component::Normal(_) | Component::CurDir))
            || target.is_absolute()
        {
            return Err(io::Error::other("Invalid registry archive link."));
        }
        // Create directories before any link exists, including forward links.
        fs::create_dir_all(
            root.join(name)
                .parent()
                .ok_or_else(|| io::Error::other("Invalid registry link path."))?,
        )?;
    }
    for (name, target, hard) in &links {
        let link = root.join(name);
        let parent = link
            .parent()
            .ok_or_else(|| io::Error::other("Invalid registry link path."))?;
        let mut dest = if *hard {
            root.to_owned()
        } else {
            parent.to_owned()
        };
        for component in target.components() {
            match component {
                Component::Normal(p) => dest.push(p),
                Component::CurDir => (),
                Component::ParentDir if dest != root => {
                    dest.pop();
                }
                _ => return Err(io::Error::other("Registry link leaves its installation.")),
            }
        }
        if *hard {
            let real = dest.canonicalize()?;
            if !real.starts_with(root.canonicalize()?) || !real.is_file() {
                return Err(io::Error::other("Invalid registry hard link."));
            }
            fs::hard_link(real, link)?;
        } else {
            #[cfg(unix)]
            std::os::unix::fs::symlink(target, link)?;
            #[cfg(windows)]
            if dest.is_dir() {
                std::os::windows::fs::symlink_dir(target, link)?
            } else {
                std::os::windows::fs::symlink_file(target, link)?
            }
        }
    }
    let canonical = root.canonicalize()?;
    for (name, _, _) in links {
        if !root.join(name).canonicalize()?.starts_with(&canonical) {
            return Err(io::Error::other(
                "Registry archive link does not resolve inside its installation.",
            ));
        }
    }
    Ok(())
}

fn write_entry(
    root: &Path,
    name: &Path,
    input: impl Read,
    mode: u32,
    remaining: &mut u64,
) -> io::Result<()> {
    if name
        .components()
        .any(|c| !matches!(c, Component::Normal(_) | Component::CurDir))
    {
        return Err(io::Error::other("Invalid archive path."));
    }
    let path = root.join(name);
    fs::create_dir_all(
        path.parent()
            .ok_or_else(|| io::Error::other("Missing archive directory."))?,
    )?;
    let mut out = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&path)?;
    let n = io::copy(&mut input.take(*remaining + 1), &mut out)?;
    *remaining = remaining
        .checked_sub(n)
        .ok_or_else(|| io::Error::other("Registry archive exceeds its unpacked size limit."))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        out.set_permissions(fs::Permissions::from_mode(0o600 | mode & 0o111))?;
    }
    Ok(())
}
