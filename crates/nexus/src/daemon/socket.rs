//! The Unix socket: where it lives, who may connect, how lines are framed.

use std::future::Future;
use std::io;
use std::os::unix::fs::PermissionsExt;
use std::path::Path;

use nexus_ipc::{ErrorCode, Event, EventLine, Op, Request, Response, encode, parse_request};
use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader};
use tokio::net::{UnixListener, UnixStream};
use tokio::sync::{broadcast, mpsc};

/// A request line longer than this is refused and the connection closed.
const MAX_LINE: u64 = 1024 * 1024;

/// Lines a connection may have waiting for its peer to read them. A peer that
/// falls this far behind is cut off rather than left to grow the Daemon.
const OUT_QUEUE: usize = 1024;

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
/// After a `subscribe` op, events from `events` are interleaved with responses.
pub async fn serve_connection<F, Fut, S, G>(
    stream: UnixStream,
    subscribe: S,
    answer: F,
) -> io::Result<()>
where
    S: Fn() -> (broadcast::Receiver<Event>, Vec<Event>, G),
    // Held for as long as the connection stays subscribed.
    G: Send + 'static,
    F: Fn(Request) -> Fut,
    Fut: Future<Output = Response>,
{
    let (read, mut write) = stream.into_split();
    let (out, mut lines) = mpsc::channel::<String>(OUT_QUEUE);
    // One writer, so responses and events never interleave mid-line.
    let writer = tokio::spawn(async move {
        while let Some(line) = lines.recv().await {
            write.write_all(line.as_bytes()).await?;
        }
        io::Result::Ok(())
    });
    let mut forwarder: Option<tokio::task::JoinHandle<()>> = None;
    let mut reader = BufReader::new(read);
    let result = loop {
        // A request line may carry a secret: wipe it when the line is done.
        let mut line = zeroize::Zeroizing::new(String::new());
        let n = match (&mut reader).take(MAX_LINE).read_line(&mut line).await {
            Ok(n) => n,
            Err(e) => break Err(e),
        };
        if n == 0 {
            break Ok(());
        }
        if !line.ends_with('\n') && n as u64 >= MAX_LINE {
            let resp = Response::err(None, ErrorCode::BadRequest, "request line too long");
            let _ = out.send(encode(&resp)).await;
            break Ok(());
        }
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        let resp = match parse_request(line) {
            Ok(Request {
                id,
                op: Op::Subscribe,
                ..
            }) => {
                if forwarder.is_none() {
                    let (mut rx, backlog, held) = subscribe();
                    let out = out.clone();
                    // Prompts that opened before this Front end came.
                    let ok = encode(&Response::ok(id, serde_json::json!({})));
                    if out.send(ok).await.is_err() {
                        break Ok(());
                    }
                    for event in backlog {
                        let line = encode(&EventLine {
                            v: nexus_ipc::PROTOCOL,
                            event,
                        });
                        if out.send(line).await.is_err() {
                            break;
                        }
                    }
                    let cut = writer.abort_handle();
                    forwarder = Some(tokio::spawn(async move {
                        let _held = held;
                        loop {
                            match rx.recv().await {
                                Ok(event) => {
                                    let line = encode(&EventLine {
                                        v: nexus_ipc::PROTOCOL,
                                        event,
                                    });
                                    match out.try_send(line) {
                                        Ok(()) => {}
                                        Err(mpsc::error::TrySendError::Full(_)) => {
                                            // Not reading: hang up on it.
                                            tracing::debug!(
                                                "dropping a subscriber that fell behind"
                                            );
                                            cut.abort();
                                            return;
                                        }
                                        Err(mpsc::error::TrySendError::Closed(_)) => return,
                                    }
                                }
                                Err(broadcast::error::RecvError::Lagged(_)) => {}
                                Err(broadcast::error::RecvError::Closed) => return,
                            }
                        }
                    }));
                    continue;
                }
                Response::ok(id, serde_json::json!({}))
            }
            Ok(req) => answer(req).await,
            Err(resp) => resp,
        };
        if out.send(encode(&resp)).await.is_err() {
            break Ok(());
        }
    };
    if let Some(f) = forwarder {
        f.abort();
    }
    drop(out);
    let _ = writer.await;
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::io::AsyncReadExt;

    #[tokio::test]
    async fn a_subscriber_that_never_reads_is_cut_off_not_buffered_forever() {
        let (server, mut client) = UnixStream::pair().expect("pair");
        let (tx, _) = broadcast::channel::<Event>(64);
        let rx_source = tx.clone();
        let task = tokio::spawn(serve_connection(
            server,
            move || (rx_source.subscribe(), Vec::new(), ()),
            |_| async { unreachable!("only a subscribe is sent") },
        ));
        client
            .write_all(b"{\"v\":1,\"id\":1,\"op\":\"subscribe\"}\n")
            .await
            .expect("write");
        let mut ack = [0u8; 1];
        client.read_exact(&mut ack).await.expect("ack");
        // Flood without reading: the socket buffer fills, then the queue.
        let event = Event::PortalChanged {
            portal: Some(nexus_ipc::Portal {
                network: "x".repeat(2000),
                url: "http://example.test".into(),
            }),
        };
        for _ in 0..200_000 {
            let _ = tx.send(event.clone());
            tokio::task::yield_now().await;
            if task.is_finished() {
                break;
            }
        }
        // The Daemon hung up: reading drains what was buffered, then ends.
        let mut sink = Vec::new();
        let ended = tokio::time::timeout(
            std::time::Duration::from_secs(10),
            client.read_to_end(&mut sink),
        )
        .await;
        assert!(ended.is_ok(), "the stalled subscriber was never dropped");
        drop(client);
        let _ = task.await;
    }
}
