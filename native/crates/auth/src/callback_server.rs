use std::fmt;
use std::io::{self, BufRead, BufReader, Write};
use std::net::{Ipv4Addr, SocketAddr, TcpListener, TcpStream};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver, Sender};
use std::thread;
use std::time::{Duration, Instant};

use crate::pkce::CALLBACK_PORT;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CallbackErrorCode {
    PortInUse,
    StateMismatch,
    Denied,
    Timeout,
    /// Binding failed for a reason other than the port being taken.
    Io,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CallbackError {
    pub code: CallbackErrorCode,
    pub message: String,
}

impl CallbackError {
    pub fn new(code: CallbackErrorCode, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
        }
    }
}

impl fmt::Display for CallbackError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.message)
    }
}

impl std::error::Error for CallbackError {}

#[derive(Debug, Clone)]
pub struct CallbackOptions {
    pub port: u16,
    pub host: Ipv4Addr,
    pub timeout: Duration,
}

impl Default for CallbackOptions {
    fn default() -> Self {
        Self {
            port: CALLBACK_PORT,
            host: Ipv4Addr::LOCALHOST,
            timeout: DEFAULT_TIMEOUT,
        }
    }
}

const DEFAULT_TIMEOUT: Duration = Duration::from_secs(5 * 60);
// std's listener has no accept timeout, so the accept loop polls to notice the deadline and close().
const POLL: Duration = Duration::from_millis(20);
const READ_TIMEOUT: Duration = Duration::from_secs(5);
const CLOSE_TAB_HTML: &str = "<!doctype html><p>Signed in to slopify. You can close this tab.</p>";

type Outcome = Result<String, CallbackError>;

/// A loopback listener for the one OAuth redirect. It answers the first `/callback` request and
/// closes, or closes on its own once the timeout passes. Dropping it closes it too.
pub struct CallbackServer {
    port: u16,
    code: Receiver<Outcome>,
    closed: Arc<AtomicBool>,
}

impl CallbackServer {
    pub fn start(expected_state: &str, opts: CallbackOptions) -> Result<Self, CallbackError> {
        let listener =
            TcpListener::bind(SocketAddr::from((opts.host, opts.port))).map_err(|e| {
                if e.kind() == io::ErrorKind::AddrInUse {
                    CallbackError::new(
                        CallbackErrorCode::PortInUse,
                        format!("Port {} is in use", opts.port),
                    )
                } else {
                    CallbackError::new(CallbackErrorCode::Io, e.to_string())
                }
            })?;
        let io_err = |e: io::Error| CallbackError::new(CallbackErrorCode::Io, e.to_string());
        let port = listener.local_addr().map_err(io_err)?.port();
        listener.set_nonblocking(true).map_err(io_err)?;

        let (tx, rx) = mpsc::channel();
        let closed = Arc::new(AtomicBool::new(false));
        let deadline = Instant::now() + opts.timeout;
        let expected = expected_state.to_string();
        let flag = closed.clone();
        thread::Builder::new()
            .name("slopify-auth-callback".into())
            .spawn(move || serve(listener, &expected, deadline, &flag, tx))
            .map_err(io_err)?;

        Ok(Self {
            port,
            code: rx,
            closed,
        })
    }

    pub fn port(&self) -> u16 {
        self.port
    }

    /// Blocks until the callback lands, the timeout passes, or `close()` is called.
    pub fn wait(&self) -> Result<String, CallbackError> {
        self.code.recv().unwrap_or_else(|_| Err(closed_error()))
    }

    pub fn close(&self) {
        self.closed.store(true, Ordering::SeqCst);
    }

    /// A server that never listened and already knows its outcome, for token store tests.
    #[cfg(test)]
    pub(crate) fn settled(outcome: Outcome) -> Self {
        let (tx, rx) = mpsc::channel();
        tx.send(outcome).unwrap();
        Self {
            port: CALLBACK_PORT,
            code: rx,
            closed: Arc::new(AtomicBool::new(false)),
        }
    }
}

impl Drop for CallbackServer {
    fn drop(&mut self) {
        self.close();
    }
}

fn closed_error() -> CallbackError {
    CallbackError::new(CallbackErrorCode::Timeout, "Callback server closed")
}

fn serve(
    listener: TcpListener,
    expected_state: &str,
    deadline: Instant,
    closed: &AtomicBool,
    tx: Sender<Outcome>,
) {
    let outcome = loop {
        if closed.load(Ordering::SeqCst) {
            break Err(closed_error());
        }
        if Instant::now() >= deadline {
            break Err(CallbackError::new(
                CallbackErrorCode::Timeout,
                "No callback arrived in time",
            ));
        }
        match listener.accept() {
            Ok((stream, _)) => {
                if let Some(outcome) = answer(stream, expected_state) {
                    break outcome;
                }
            }
            Err(e) if e.kind() == io::ErrorKind::WouldBlock => thread::sleep(POLL),
            Err(e) => {
                log::warn!("callback server accept failed: {e}");
                thread::sleep(POLL);
            }
        }
    };
    // Stop listening before anyone learns the outcome, so a second hit is refused.
    drop(listener);
    let _ = tx.send(outcome);
}

/// Answers one request. Returns the outcome for `/callback`, None for anything else.
fn answer(stream: TcpStream, expected_state: &str) -> Option<Outcome> {
    let target = match read_target(&stream) {
        Ok(target) => target,
        Err(e) => {
            log::warn!("callback server could not read a request: {e}");
            return None;
        }
    };
    let (path, query) = target.split_once('?').unwrap_or((&target, ""));
    let mut out = &stream;
    if path != "/callback" {
        let _ = out
            .write_all(b"HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\nConnection: close\r\n\r\n");
        return None;
    }
    let head = format!(
        "HTTP/1.1 200 OK\r\nContent-Type: text/html; charset=utf-8\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
        CLOSE_TAB_HTML.len()
    );
    let _ = out.write_all(head.as_bytes());
    let _ = out.write_all(CLOSE_TAB_HTML.as_bytes());
    let _ = out.flush();
    Some(parse_callback(query, expected_state))
}

/// Reads the request line and drains the headers so closing the socket doesn't reset the
/// connection under the browser. Returns the request target.
fn read_target(stream: &TcpStream) -> io::Result<String> {
    stream.set_nonblocking(false)?;
    stream.set_read_timeout(Some(READ_TIMEOUT))?;
    let mut reader = BufReader::new(stream);
    let mut line = String::new();
    reader.read_line(&mut line)?;
    let target = line
        .split_whitespace()
        .nth(1)
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "malformed request line"))?
        .to_string();
    loop {
        let mut header = String::new();
        if reader.read_line(&mut header)? == 0 || header.trim_end().is_empty() {
            break;
        }
    }
    Ok(target)
}

fn parse_callback(query: &str, expected_state: &str) -> Outcome {
    let get = |key: &str| {
        form_urlencoded::parse(query.as_bytes())
            .find(|(k, _)| k == key)
            .map(|(_, v)| v.into_owned())
    };
    if get("state").as_deref() != Some(expected_state) {
        return Err(CallbackError::new(
            CallbackErrorCode::StateMismatch,
            "Callback state did not match",
        ));
    }
    if let Some(denied) = get("error").filter(|e| !e.is_empty()) {
        return Err(CallbackError::new(
            CallbackErrorCode::Denied,
            format!("Spotify returned {denied}"),
        ));
    }
    match get("code").filter(|c| !c.is_empty()) {
        Some(code) => Ok(code),
        None => Err(CallbackError::new(
            CallbackErrorCode::Denied,
            "Callback carried no code",
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Read;

    fn start(state: &str) -> CallbackServer {
        start_with(state, DEFAULT_TIMEOUT)
    }

    fn start_with(state: &str, timeout: Duration) -> CallbackServer {
        CallbackServer::start(
            state,
            CallbackOptions {
                port: 0,
                timeout,
                ..Default::default()
            },
        )
        .unwrap()
    }

    struct Reply {
        status: u16,
        head: String,
        body: String,
    }

    fn hit(port: u16, path: &str) -> io::Result<Reply> {
        let mut stream = TcpStream::connect((Ipv4Addr::LOCALHOST, port))?;
        write!(
            stream,
            "GET {path} HTTP/1.1\r\nHost: 127.0.0.1\r\nConnection: close\r\n\r\n"
        )?;
        let mut raw = String::new();
        stream.read_to_string(&mut raw)?;
        let (head, body) = raw.split_once("\r\n\r\n").unwrap_or((&raw, ""));
        let status = head
            .split_whitespace()
            .nth(1)
            .and_then(|s| s.parse().ok())
            .ok_or_else(|| io::Error::other(format!("no status in {raw:?}")))?;
        Ok(Reply {
            status,
            head: head.to_ascii_lowercase(),
            body: body.to_string(),
        })
    }

    fn code_of(r: Outcome) -> CallbackErrorCode {
        r.unwrap_err().code
    }

    #[test]
    fn resolves_with_the_code_when_the_state_matches_and_tells_the_browser_to_close_the_tab() {
        let s = start("good");
        let res = hit(s.port(), "/callback?code=abc&state=good").unwrap();
        assert_eq!(res.status, 200);
        assert!(res.head.contains("content-type: text/html"));
        assert!(res.body.to_lowercase().contains("close"));
        assert_eq!(s.wait(), Ok("abc".into()));
    }

    #[test]
    fn rejects_state_mismatch_on_the_wrong_state() {
        let s = start("good");
        hit(s.port(), "/callback?code=abc&state=evil").unwrap();
        assert_eq!(code_of(s.wait()), CallbackErrorCode::StateMismatch);
    }

    #[test]
    fn rejects_denied_when_spotify_sends_error() {
        let s = start("good");
        hit(s.port(), "/callback?error=access_denied&state=good").unwrap();
        let err = s.wait().unwrap_err();
        assert_eq!(err.code, CallbackErrorCode::Denied);
        assert_eq!(err.message, "Spotify returned access_denied");
    }

    #[test]
    fn rejects_denied_when_the_callback_carries_no_code() {
        let s = start("good");
        hit(s.port(), "/callback?state=good").unwrap();
        assert_eq!(code_of(s.wait()), CallbackErrorCode::Denied);
    }

    #[test]
    fn decodes_percent_encoded_query_values() {
        let s = start("a/b=c");
        hit(s.port(), "/callback?code=x%2By&state=a%2Fb%3Dc").unwrap();
        assert_eq!(s.wait(), Ok("x+y".into()));
    }

    #[test]
    fn answers_404_on_other_paths_and_keeps_waiting() {
        let s = start("good");
        assert_eq!(hit(s.port(), "/favicon.ico").unwrap().status, 404);
        let ok = hit(s.port(), "/callback?code=later&state=good").unwrap();
        assert_eq!(ok.status, 200);
        assert_eq!(s.wait(), Ok("later".into()));
    }

    #[test]
    fn closes_after_the_first_callback() {
        let s = start("good");
        hit(s.port(), "/callback?code=abc&state=good").unwrap();
        s.wait().unwrap();
        assert!(hit(s.port(), "/callback?code=again&state=good").is_err());
    }

    #[test]
    fn rejects_port_in_use_when_the_port_is_taken() {
        let first = start("a");
        let err = CallbackServer::start(
            "b",
            CallbackOptions {
                port: first.port(),
                ..Default::default()
            },
        )
        .err()
        .unwrap();
        assert_eq!(err.code, CallbackErrorCode::PortInUse);
        assert_eq!(err.message, format!("Port {} is in use", first.port()));
    }

    #[test]
    fn rejects_timeout_when_nobody_calls_back() {
        let s = start_with("good", Duration::from_millis(20));
        assert_eq!(code_of(s.wait()), CallbackErrorCode::Timeout);
    }

    #[test]
    fn close_ends_the_wait_and_stops_listening() {
        let s = start("good");
        let port = s.port();
        s.close();
        assert_eq!(code_of(s.wait()), CallbackErrorCode::Timeout);
        assert!(hit(port, "/callback?code=abc&state=good").is_err());
    }
}
