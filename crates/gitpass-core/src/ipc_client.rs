//! Client side of the `gitpassd` socket, used by the Git hooks.
//!
//! A hook must not block a commit just because the daemon is restarting (launchd `kickstart` on app
//! start or update, `KeepAlive` after a crash) or busy asking GitHub or the Keychain: connecting is
//! retried for a few seconds while the socket is missing or refuses connections, a connection dropped
//! mid-request is retried once, and the reply may take longer than the daemon's own deadline
//! ([`DAEMON_VALIDATE_DEADLINE`]).

use std::path::Path;
use std::time::Duration;

use crate::IpcRequest;
#[cfg(unix)]
use crate::{decode_response_line, encode_line};
use crate::IpcResponse;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct IpcTimeouts {
    /// How long to keep retrying while the socket is missing or refuses connections.
    pub connect_wait: Duration,
    /// How long to wait for the reply once connected.
    pub reply: Duration,
}

impl Default for IpcTimeouts {
    fn default() -> Self {
        Self {
            connect_wait: Duration::from_secs(3),
            reply: Duration::from_secs(25),
        }
    }
}

/// The daemon answers a hook validation within this time, even when GitHub or the Keychain is slow.
pub const DAEMON_VALIDATE_DEADLINE: Duration = Duration::from_secs(15);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DaemonUnreachable {
    /// No socket, or nobody accepted the connection within `connect_wait`.
    NotRunning,
    /// Connected, but no reply within `reply`.
    Slow,
    /// The reply could not be understood.
    BadReply,
}

#[cfg(unix)]
enum Attempt {
    Connect,
    Dropped,
    Timeout,
    Bad,
}

#[cfg(unix)]
fn exchange(socket: &Path, line: &str, reply: Duration) -> Result<IpcResponse, Attempt> {
    use std::io::{BufRead, BufReader, ErrorKind, Write};
    use std::os::unix::net::UnixStream;

    let mut stream = UnixStream::connect(socket).map_err(|_| Attempt::Connect)?;
    let _ = stream.set_read_timeout(Some(reply));
    let _ = stream.set_write_timeout(Some(reply));
    stream.write_all(line.as_bytes()).map_err(|_| Attempt::Dropped)?;
    let mut buf = String::new();
    match BufReader::new(stream).read_line(&mut buf) {
        Ok(0) => Err(Attempt::Dropped),
        Ok(_) => decode_response_line(&buf).map_err(|_| Attempt::Bad),
        Err(e) if matches!(e.kind(), ErrorKind::WouldBlock | ErrorKind::TimedOut) => Err(Attempt::Timeout),
        Err(_) => Err(Attempt::Dropped),
    }
}

/// Sends one request to the daemon at `socket` and waits for its reply (see the module docs).
#[cfg(unix)]
pub fn request(socket: &Path, req: &IpcRequest, t: IpcTimeouts) -> Result<IpcResponse, DaemonUnreachable> {
    use std::time::Instant;

    let line = encode_line(req).map_err(|_| DaemonUnreachable::BadReply)?;
    let mut deadline = Instant::now() + t.connect_wait;
    let mut retried_drop = false;
    loop {
        match exchange(socket, &line, t.reply) {
            Ok(resp) => return Ok(resp),
            Err(Attempt::Connect) if Instant::now() < deadline => std::thread::sleep(Duration::from_millis(100)),
            Err(Attempt::Connect) => return Err(DaemonUnreachable::NotRunning),
            // The daemon went away mid-request (restart); validating is idempotent, so ask the new one.
            Err(Attempt::Dropped) if !retried_drop => {
                retried_drop = true;
                deadline = deadline.max(Instant::now() + t.connect_wait);
            }
            Err(Attempt::Dropped) => return Err(DaemonUnreachable::NotRunning),
            Err(Attempt::Timeout) => return Err(DaemonUnreachable::Slow),
            Err(Attempt::Bad) => return Err(DaemonUnreachable::BadReply),
        }
    }
}

#[cfg(not(unix))]
pub fn request(_socket: &Path, _req: &IpcRequest, _t: IpcTimeouts) -> Result<IpcResponse, DaemonUnreachable> {
    Err(DaemonUnreachable::NotRunning)
}

/// A daemon is accepting connections on `socket` (a second `gitpassd` must not replace its socket).
#[cfg(unix)]
pub fn socket_in_use(socket: &Path) -> bool {
    std::os::unix::net::UnixStream::connect(socket).is_ok()
}

#[cfg(not(unix))]
pub fn socket_in_use(_socket: &Path) -> bool {
    false
}

/// Runs `f` on its own thread and returns its result, or `on_timeout()` once `deadline` passes
/// (then `f` finishes detached and its result is dropped).
pub fn with_deadline<T: Send + 'static>(
    deadline: Duration,
    f: impl FnOnce() -> T + Send + 'static,
    on_timeout: impl FnOnce() -> T,
) -> T {
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let _ = tx.send(f());
    });
    rx.recv_timeout(deadline).unwrap_or_else(|_| on_timeout())
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use std::io::{BufRead, BufReader, Write};
    use std::os::unix::net::UnixListener;
    use std::path::PathBuf;
    use std::time::Instant;

    struct Sock(PathBuf);

    impl Drop for Sock {
        fn drop(&mut self) {
            let _ = std::fs::remove_file(&self.0);
        }
    }

    /// Short path: Unix socket paths are limited to ~104 bytes on macOS.
    fn sock() -> Sock {
        Sock(PathBuf::from(format!("/tmp/gp-{}.sock", &uuid::Uuid::new_v4().simple().to_string()[..12])))
    }

    fn pong() -> String {
        crate::encode_line(&IpcResponse::Pong { gitpassd_version: "t".into() }).unwrap()
    }

    /// Serves `n` connections; each waits `delay` before answering (or closes when `drop_first`).
    fn serve(listener: UnixListener, n: usize, delay: Duration, drop_first: bool) -> std::thread::JoinHandle<()> {
        std::thread::spawn(move || {
            for i in 0..n {
                let Ok((stream, _)) = listener.accept() else { return };
                let mut line = String::new();
                let mut reader = BufReader::new(stream.try_clone().unwrap());
                let _ = reader.read_line(&mut line);
                if drop_first && i == 0 {
                    continue;
                }
                std::thread::sleep(delay);
                let _ = (&stream).write_all(pong().as_bytes());
            }
        })
    }

    fn quick(connect_ms: u64, reply_ms: u64) -> IpcTimeouts {
        IpcTimeouts {
            connect_wait: Duration::from_millis(connect_ms),
            reply: Duration::from_millis(reply_ms),
        }
    }

    #[test]
    fn waits_for_a_daemon_that_is_still_starting() {
        let s = sock();
        let path = s.0.clone();
        let starter = std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(400));
            serve(UnixListener::bind(&path).unwrap(), 1, Duration::ZERO, false).join().unwrap();
        });
        let got = request(&s.0, &IpcRequest::Ping, quick(3000, 2000));
        assert!(matches!(got, Ok(IpcResponse::Pong { .. })), "{got:?}");
        starter.join().unwrap();
    }

    #[test]
    fn stale_socket_of_a_restarting_daemon_is_retried() {
        let s = sock();
        drop(UnixListener::bind(&s.0).unwrap());
        assert!(s.0.exists() && !socket_in_use(&s.0));
        let path = s.0.clone();
        let restarter = std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(300));
            let _ = std::fs::remove_file(&path);
            serve(UnixListener::bind(&path).unwrap(), 1, Duration::ZERO, false).join().unwrap();
        });
        assert!(request(&s.0, &IpcRequest::Ping, quick(3000, 2000)).is_ok());
        restarter.join().unwrap();
    }

    #[test]
    fn missing_daemon_is_reported_after_the_wait() {
        let s = sock();
        let started = Instant::now();
        assert_eq!(request(&s.0, &IpcRequest::Ping, quick(300, 1000)).unwrap_err(), DaemonUnreachable::NotRunning);
        assert!(started.elapsed() >= Duration::from_millis(300));
    }

    #[test]
    fn slow_reply_within_the_window_is_accepted_and_beyond_it_is_slow() {
        let s = sock();
        let h = serve(UnixListener::bind(&s.0).unwrap(), 2, Duration::from_millis(600), false);
        assert!(request(&s.0, &IpcRequest::Ping, quick(500, 3000)).is_ok());
        assert_eq!(request(&s.0, &IpcRequest::Ping, quick(500, 100)).unwrap_err(), DaemonUnreachable::Slow);
        h.join().unwrap();
    }

    #[test]
    fn connection_dropped_mid_request_is_retried_once() {
        let s = sock();
        let h = serve(UnixListener::bind(&s.0).unwrap(), 2, Duration::ZERO, true);
        assert!(request(&s.0, &IpcRequest::Ping, quick(500, 1000)).is_ok());
        h.join().unwrap();
    }

    #[test]
    fn deadline_returns_the_fallback_when_work_is_slow() {
        let slow = with_deadline(Duration::from_millis(50), || {
            std::thread::sleep(Duration::from_millis(500));
            1
        }, || 2);
        assert_eq!(slow, 2);
        assert_eq!(with_deadline(Duration::from_secs(2), || 1, || 2), 1);
    }
}
