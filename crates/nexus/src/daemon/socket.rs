//! The Unix socket: where it lives, who may connect, how lines are framed.

use std::future::Future;
use std::io;
use std::os::unix::fs::PermissionsExt;
use std::path::Path;

use nexus_ipc::{ErrorCode, Request, Response, encode, parse_request};
use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader};
use tokio::net::{UnixListener, UnixStream};

/// A request line longer than this is refused and the connection closed.
const MAX_LINE: u64 = 1024 * 1024;

/// The socket to accept on: the one systemd passed us (`nexus.socket`), else
/// a fresh one at `$XDG_RUNTIME_DIR/nexus/nexus.sock`.
pub fn listen() -> io::Result<UnixListener> {
    if let Some(inherited) = listenfd::ListenFd::from_env().take_unix_listener(0)? {
        // systemd owns the path, the 0700 directory and the socket mode.
        tracing::info!("using the socket passed by systemd");
        inherited.set_nonblocking(true)?;
        return UnixListener::from_std(inherited);
    }
    let path = nexus_ipc::socket_path()
        .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "XDG_RUNTIME_DIR is not set"))?;
    bind(&path)
}

fn bind(path: &Path) -> io::Result<UnixListener> {
    let dir = path.parent().expect("socket path has a parent");
    std::fs::create_dir_all(dir)?;
    // The private directory is the first gate; the uid check is the second.
    std::fs::set_permissions(dir, std::fs::Permissions::from_mode(0o700))?;
    if std::os::unix::net::UnixStream::connect(path).is_ok() {
        return Err(io::Error::new(
            io::ErrorKind::AddrInUse,
            format!("another Daemon is already listening on {}", path.display()),
        ));
    }
    match std::fs::remove_file(path) {
        Ok(()) => {}
        Err(e) if e.kind() == io::ErrorKind::NotFound => {}
        Err(e) => return Err(e),
    }
    let listener = UnixListener::bind(path)?;
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600))?;
    Ok(listener)
}

/// Whether the peer runs as the same user as the Daemon.
pub fn same_user(stream: &UnixStream) -> bool {
    stream
        .peer_cred()
        .is_ok_and(|c| c.uid() == rustix::process::getuid().as_raw())
}

/// Reads request lines until the peer hangs up, answering each in order.
pub async fn serve_connection<F, Fut>(stream: UnixStream, answer: F) -> io::Result<()>
where
    F: Fn(Request) -> Fut,
    Fut: Future<Output = Response>,
{
    let (read, mut write) = stream.into_split();
    let mut reader = BufReader::new(read);
    loop {
        let mut line = String::new();
        let n = (&mut reader).take(MAX_LINE).read_line(&mut line).await?;
        if n == 0 {
            return Ok(());
        }
        if !line.ends_with('\n') && n as u64 >= MAX_LINE {
            let resp = Response::err(None, ErrorCode::BadRequest, "request line too long");
            write.write_all(encode(&resp).as_bytes()).await?;
            return Ok(());
        }
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        let resp = match parse_request(line) {
            Ok(req) => answer(req).await,
            Err(resp) => resp,
        };
        write.write_all(encode(&resp).as_bytes()).await?;
    }
}
