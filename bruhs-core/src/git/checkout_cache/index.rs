//! Refresh the stat fields in our private Git v2 index after APFS assigns new
//! inodes to cloned files. Git's object IDs and executable bits stay untouched.
use sha1::{Digest, Sha1};
use std::{
    ffi::OsStr,
    fs,
    os::unix::{ffi::OsStrExt, fs::MetadataExt},
    path::{Component, Path},
};

fn number(bytes: &[u8], at: usize) -> Option<u32> {
    Some(u32::from_be_bytes(bytes.get(at..at + 4)?.try_into().ok()?))
}

pub(super) fn for_clone(source: &Path, root: &Path) -> Option<Vec<u8>> {
    let mut index = fs::read(source).ok()?;
    let end = index.len().checked_sub(20)?;
    let (body, checksum) = index.split_at_mut(end);
    if body.get(..4)? != b"DIRC" || number(body, 4)? != 2 || Sha1::digest(&*body)[..] != *checksum {
        return None;
    }
    let mut offset = 12;
    for _ in 0..number(body, 8)? {
        // v2 entries are eight-byte aligned relative to the start of each entry.
        let fixed = body.get(offset..offset + 62)?;
        let flags = u16::from_be_bytes(fixed[60..62].try_into().ok()?);
        if flags & 0xf000 != 0 {
            return None;
        }
        let start = offset + 62;
        let size = usize::from(flags & 0xfff);
        let length = if size == 0xfff {
            body.get(start..)?.iter().position(|&c| c == 0)?
        } else {
            size
        };
        let name = body.get(start..start + length)?;
        let path = Path::new(OsStr::from_bytes(name));
        if name.is_empty() || body.get(start+length)!=Some(&0) || path.components().any(|part|
            !matches!(part,Component::Normal(s) if !s.as_bytes().eq_ignore_ascii_case(b".git"))) {
            return None;
        }
        let stat = fs::symlink_metadata(root.join(path)).ok()?;
        let kind = if stat.is_dir() && number(fixed, 24)? == 0o160000 {
            if fs::read_dir(root.join(path)).ok()?.next().is_some() {
                return None;
            }
            0o160000
        } else if stat.is_symlink() {
            0o120000
        } else if stat.is_file() {
            if stat.mode() & 0o100 != 0 {
                0o100755
            } else {
                0o100644
            }
        } else {
            return None;
        };
        let mut bytes = stat.size() as u32;
        if bytes == 0 && stat.size() != 0 {
            bytes = 0x80000000;
        }
        if kind != number(fixed, 24)?
            || (kind != 0o160000
                && (bytes != number(fixed, 36)?
                    || stat.mtime() as u32 != number(fixed, 8)?
                    || stat.mtime_nsec() as u32 != number(fixed, 12)?))
        {
            return None;
        }
        // Clonefile preserves the content, mode and mtime verified above.
        for (field, value) in [
            (0, stat.ctime() as u32),
            (4, stat.ctime_nsec() as u32),
            (8, stat.mtime() as u32),
            (12, stat.mtime_nsec() as u32),
            (36, bytes),
            (16, stat.dev() as u32),
            (20, stat.ino() as u32),
            (28, stat.uid()),
            (32, stat.gid()),
        ] {
            body[offset + field..offset + field + 4].copy_from_slice(&value.to_be_bytes());
        }
        offset += (62 + length + 1).next_multiple_of(8);
    }
    // The cache writer requests a full, unsplit index. Only Git's optional
    // cached-tree extension is independent of the filesystem stat records.
    while offset < body.len() {
        if body.get(offset..offset + 4)? != b"TREE" {
            return None;
        }
        offset = offset.checked_add(8 + number(body, offset + 4)? as usize)?;
    }
    if offset != body.len() {
        return None;
    }
    checksum.copy_from_slice(&Sha1::digest(&*body));
    Some(index)
}
