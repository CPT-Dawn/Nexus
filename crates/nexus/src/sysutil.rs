//! Small process and file helpers the Daemon shares.

use std::io;
use std::path::Path;
use std::process::{Command, Stdio};

/// Runs `command` with no input and no output, and reaps it on a thread when
/// it ends, so a closed browser or terminal leaves no zombie.
pub fn spawn_detached(command: &mut Command) -> io::Result<()> {
    let mut child = command
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()?;
    std::thread::spawn(move || child.wait());
    Ok(())
}

/// Writes `text` whole beside `path` and renames it over the file, creating
/// the directory first, so a crash never leaves half a file.
pub fn write_atomically(path: &Path, text: &str) -> io::Result<()> {
    let dir = path.parent().expect("path has a parent");
    std::fs::create_dir_all(dir)?;
    let name = path.file_name().expect("path names a file");
    let tmp = dir.join(format!(".{}.nexus-tmp", name.to_string_lossy()));
    std::fs::write(&tmp, text)?;
    std::fs::rename(&tmp, path)
}
