use super::Connection;
use serde::Serialize;
use std::{
    fs,
    io::{self, Read, Write},
    os::unix::fs::{DirBuilderExt, OpenOptionsExt, PermissionsExt},
    path::{Path, PathBuf},
};
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};

pub(super) fn private_directory() -> io::Result<PathBuf> {
    // Keep Unix socket paths below macOS's 104-byte limit. No user state lives here.
    let directory = PathBuf::from("/tmp").join(format!("bruhs-bridge-{}", uuid::Uuid::new_v4()));
    fs::DirBuilder::new().mode(0o700).create(&directory)?;
    Ok(directory)
}

pub(super) fn restrict_permissions(path: &Path) -> io::Result<()> {
    fs::set_permissions(path, fs::Permissions::from_mode(0o600))
}

pub(super) fn write_connection(path: &Path, connection: &Connection) -> io::Result<()> {
    let mut file = fs::OpenOptions::new()
        .create_new(true)
        .write(true)
        .mode(0o600)
        .open(path)?;
    file.write_all(&serde_json::to_vec(connection)?)?;
    file.sync_all()
}

pub(super) fn read_connection(path: &Path) -> io::Result<Connection> {
    let metadata = fs::symlink_metadata(path)?;
    if !metadata.is_file() || metadata.permissions().mode() & 0o077 != 0 || metadata.len() > 4096 {
        return Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            "Expected a private workspace connection file.",
        ));
    }
    let file = fs::File::open(path)?;
    let opened = file.metadata()?;
    if !opened.is_file() || opened.permissions().mode() & 0o077 != 0 {
        return Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            "Workspace connection file changed.",
        ));
    }
    let mut bytes = Vec::new();
    file.take(4097).read_to_end(&mut bytes)?;
    let connection: Connection = serde_json::from_slice(&bytes)?;
    if bytes.len() > 4096 || connection.token.len() != 64 || !connection.socket.is_absolute() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "Invalid workspace connection file.",
        ));
    }
    Ok(connection)
}

pub(super) fn remove_session_files(directory: &Path) {
    // Never recursively remove a caller-supplied directory.
    let _ = fs::remove_file(directory.join("connection.json"));
    let _ = fs::remove_file(directory.join("bridge.sock"));
    let _ = fs::remove_dir(directory);
}

pub(super) fn same_token(supplied: &str, expected: &str) -> bool {
    supplied.len() == expected.len()
        && supplied
            .as_bytes()
            .iter()
            .zip(expected.as_bytes())
            .fold(0u8, |diff, (left, right)| diff | (left ^ right))
            == 0
}

pub(super) async fn read_frame(
    stream: &mut (impl AsyncRead + Unpin),
    limit: usize,
) -> io::Result<Vec<u8>> {
    let length = stream.read_u32().await? as usize;
    if length == 0 || length > limit {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "Workspace frame exceeds its limit.",
        ));
    }
    let mut bytes = vec![0; length];
    stream.read_exact(&mut bytes).await?;
    Ok(bytes)
}

pub(super) async fn write_frame(
    stream: &mut (impl AsyncWrite + Unpin),
    value: &impl Serialize,
    limit: usize,
) -> io::Result<()> {
    let bytes = serde_json::to_vec(value)?;
    let len = u32::try_from(bytes.len())
        .ok()
        .filter(|_| bytes.len() <= limit)
        .ok_or_else(|| {
            io::Error::new(
                io::ErrorKind::InvalidData,
                "Workspace response exceeds its limit.",
            )
        })?;
    stream.write_u32(len).await?;
    stream.write_all(&bytes).await?;
    stream.flush().await
}
