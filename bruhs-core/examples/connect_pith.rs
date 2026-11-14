//! Connect the scanner selected by the operator, using the same JSON Pith's settings copy.
use std::{io, path::PathBuf};

fn main() -> io::Result<()> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let flag = |name| {
        args.windows(2)
            .find(|pair| pair[0] == name)
            .map(|pair| PathBuf::from(&pair[1]))
    };
    let command = flag("--command").ok_or_else(|| io::Error::other("Usage: connect_pith --command /path/to/storage-scanner --root /allowed/folder [--config /path/to/pith-mcp.json]"))?;
    let root =
        flag("--root").ok_or_else(|| io::Error::other("--root /allowed/folder is required."))?;
    if !command.is_file() || !root.is_dir() {
        return Err(io::Error::other(
            "Choose an existing scanner executable and folder.",
        ));
    }
    let connection = bruhs_core::pith::Connection::new(command, root)?;
    let path = flag("--config").map_or_else(bruhs_core::pith::default_file, Ok)?;
    connection.save(&path)?;
    println!(
        "Connected Pith for the next bruhs-ai turn.\nFolder: {}\nConfiguration: {}",
        connection.root().display(),
        path.display()
    );
    Ok(())
}
