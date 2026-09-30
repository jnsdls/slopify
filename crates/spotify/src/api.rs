use std::fmt;
use std::sync::Arc;
use std::time::Duration;

use serde_json::Value;

use crate::types::BridgeError;

pub type BoxError = Box<dyn std::error::Error + Send + Sync>;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Method {
    Get,
    Put,
    Post,
    Delete,
}

impl Method {
    pub fn as_str(self) -> &'static str {
        match self {
            Method::Get => "GET",
            Method::Put => "PUT",
            Method::Post => "POST",
            Method::Delete => "DELETE",
        }
    }
}

impl fmt::Display for Method {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// A non-2xx the retries did not fix.
#[derive(Debug, Clone, PartialEq)]
pub struct ApiError {
    pub method: Method,
    pub path: String,
    pub status: u16,
    pub body: Option<Value>,
}

impl fmt::Display for ApiError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{} {} {}", self.method, self.path, self.status)
    }
}

impl std::error::Error for ApiError {}

#[derive(Debug)]
pub enum Error {
    Api(ApiError),
    /// A failure the UI switches on (bad-link, not-found, play-failed, ...).
    Bridge(BridgeError),
    /// The token source could not produce a token.
    Token(BoxError),
    /// The request never got a status: DNS, TLS, timeout, connection reset.
    Transport(BoxError),
    /// Spotify answered 2xx with JSON in a shape we do not understand.
    Decode(serde_json::Error),
}

impl Error {
    pub fn api_status(&self) -> Option<u16> {
        match self {
            Error::Api(e) => Some(e.status),
            _ => None,
        }
    }
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Error::Api(e) => e.fmt(f),
            Error::Bridge(e) => e.fmt(f),
            Error::Token(e) => write!(f, "access token: {e}"),
            Error::Transport(e) => write!(f, "transport: {e}"),
            Error::Decode(e) => write!(f, "decode: {e}"),
        }
    }
}

impl std::error::Error for Error {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Error::Api(e) => Some(e),
            Error::Bridge(e) => Some(e),
            Error::Token(e) | Error::Transport(e) => Some(e.as_ref()),
            Error::Decode(e) => Some(e),
        }
    }
}

impl From<ApiError> for Error {
    fn from(e: ApiError) -> Self {
        Error::Api(e)
    }
}

impl From<BridgeError> for Error {
    fn from(e: BridgeError) -> Self {
        Error::Bridge(e)
    }
}

/// Where access tokens come from. `force_refresh` is called once when Spotify answers 401.
pub trait TokenSource: Send + Sync {
    fn access_token(&self) -> Result<String, BoxError>;
    fn force_refresh(&self) -> Result<String, BoxError>;
}

/// A `TokenSource` from two closures, so the app can wire in the auth crate without either
/// crate depending on the other.
pub struct TokenFns<G, R> {
    pub get: G,
    pub refresh: R,
}

impl<G, R> TokenSource for TokenFns<G, R>
where
    G: Fn() -> Result<String, BoxError> + Send + Sync,
    R: Fn() -> Result<String, BoxError> + Send + Sync,
{
    fn access_token(&self) -> Result<String, BoxError> {
        (self.get)()
    }

    fn force_refresh(&self) -> Result<String, BoxError> {
        (self.refresh)()
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HttpRequest {
    pub method: Method,
    pub url: String,
    pub headers: Vec<(&'static str, String)>,
    pub body: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct HttpResponse {
    pub status: u16,
    pub body: String,
    pub retry_after: Option<String>,
}

/// One blocking round trip. Returns `Err` only when there is no status to report; every status,
/// 4xx and 5xx included, comes back as `Ok`.
pub trait HttpClient: Send + Sync {
    fn send(&self, req: &HttpRequest) -> Result<HttpResponse, BoxError>;
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct RequestOpts {
    pub query: Vec<(String, String)>,
    pub body: Option<Value>,
}

impl RequestOpts {
    pub fn query(mut self, key: &str, value: &str) -> Self {
        self.query.push((key.to_owned(), value.to_owned()));
        self
    }

    pub fn body(mut self, body: Value) -> Self {
        self.body = Some(body);
        self
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct ApiResponse {
    pub status: u16,
    pub json: Option<Value>,
}

/// What playback and sources need from the Web API. `SpotifyApi` is the real one; tests fake it.
pub trait WebApi {
    /// `path` is either a `/v1`-relative path or an absolute URL. The absolute form exists so
    /// callers can follow the `next` links Spotify returns in paged responses.
    fn request(&self, method: Method, path: &str, opts: RequestOpts) -> Result<ApiResponse, Error>;
}

impl<T: WebApi + ?Sized> WebApi for &T {
    fn request(&self, method: Method, path: &str, opts: RequestOpts) -> Result<ApiResponse, Error> {
        (**self).request(method, path, opts)
    }
}

impl<T: WebApi + ?Sized> WebApi for Arc<T> {
    fn request(&self, method: Method, path: &str, opts: RequestOpts) -> Result<ApiResponse, Error> {
        (**self).request(method, path, opts)
    }
}

const BASE: &str = "https://api.spotify.com/v1";

type Sleep = Box<dyn Fn(Duration) + Send + Sync>;

/// Blocking Web API client. Call it from a background thread.
pub struct SpotifyApi {
    tokens: Box<dyn TokenSource>,
    http: Box<dyn HttpClient>,
    sleep: Sleep,
}

impl SpotifyApi {
    pub fn new(tokens: impl TokenSource + 'static, http: impl HttpClient + 'static) -> Self {
        Self {
            tokens: Box::new(tokens),
            http: Box::new(http),
            sleep: Box::new(std::thread::sleep),
        }
    }

    /// Replaces the wait before a 429 or 5xx retry. Tests use it to skip the real sleep.
    pub fn with_sleep(mut self, sleep: impl Fn(Duration) + Send + Sync + 'static) -> Self {
        self.sleep = Box::new(sleep);
        self
    }

    fn send(
        &self,
        method: Method,
        url: &str,
        token: &str,
        body: Option<&str>,
    ) -> Result<HttpResponse, Error> {
        let mut headers = vec![("Authorization", format!("Bearer {token}"))];
        if body.is_some() {
            headers.push(("Content-Type", "application/json".to_owned()));
        }
        let req = HttpRequest {
            method,
            url: url.to_owned(),
            headers,
            body: body.map(str::to_owned),
        };
        self.http.send(&req).map_err(Error::Transport)
    }
}

impl WebApi for SpotifyApi {
    fn request(&self, method: Method, path: &str, opts: RequestOpts) -> Result<ApiResponse, Error> {
        let url = build_url(path, &opts.query);
        let body = opts.body.as_ref().map(Value::to_string);

        let mut token = self.tokens.access_token().map_err(Error::Token)?;
        let mut refreshed = false;
        let mut waited = false;

        loop {
            let res = self.send(method, &url, &token, body.as_deref())?;
            if (200..300).contains(&res.status) {
                return Ok(ApiResponse {
                    status: res.status,
                    json: parse_json(&res.body),
                });
            }

            log::warn!("{method} {path} {}", res.status);

            if res.status == 401 && !refreshed {
                refreshed = true;
                token = self.tokens.force_refresh().map_err(Error::Token)?;
                continue;
            }
            if res.status == 429 && !waited {
                waited = true;
                (self.sleep)(retry_after(res.retry_after.as_deref()));
                continue;
            }
            if res.status >= 500 && !waited {
                waited = true;
                (self.sleep)(Duration::from_secs(1));
                continue;
            }
            return Err(Error::Api(ApiError {
                method,
                path: path.to_owned(),
                status: res.status,
                body: parse_json(&res.body),
            }));
        }
    }
}

fn build_url(path: &str, query: &[(String, String)]) -> String {
    let url = if path.starts_with("https://") {
        path.to_owned()
    } else {
        format!("{BASE}{path}")
    };
    if query.is_empty() {
        return url;
    }
    // Same as URLSearchParams.set: parse what is there, replace or append each key, reserialise.
    let (base, existing) = url.split_once('?').unwrap_or((&url, ""));
    let mut pairs: Vec<(String, String)> = form_urlencoded::parse(existing.as_bytes())
        .map(|(k, v)| (k.into_owned(), v.into_owned()))
        .collect();
    for (k, v) in query {
        let mut seen = false;
        pairs.retain_mut(|(pk, pv)| {
            if pk != k {
                return true;
            }
            if seen {
                return false;
            }
            seen = true;
            v.clone_into(pv);
            true
        });
        if !seen {
            pairs.push((k.clone(), v.clone()));
        }
    }
    let qs = form_urlencoded::Serializer::new(String::new())
        .extend_pairs(&pairs)
        .finish();
    format!("{base}?{qs}")
}

fn retry_after(header: Option<&str>) -> Duration {
    match header.and_then(|h| h.trim().parse::<f64>().ok()) {
        Some(s) if s.is_finite() && s > 0.0 => Duration::from_secs_f64(s),
        _ => Duration::from_secs(1),
    }
}

// Spotify answers some PUTs (shuffle, repeat) with 200 and a bare token where the docs say 204.
fn parse_json(text: &str) -> Option<Value> {
    if text.is_empty() {
        return None;
    }
    serde_json::from_str(text).ok()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_log;
    use serde_json::json;
    use std::collections::VecDeque;
    use std::sync::Mutex;

    #[derive(Default)]
    struct Fake {
        replies: Mutex<VecDeque<HttpResponse>>,
        calls: Mutex<Vec<HttpRequest>>,
    }

    impl HttpClient for Arc<Fake> {
        fn send(&self, req: &HttpRequest) -> Result<HttpResponse, BoxError> {
            self.calls.lock().unwrap().push(req.clone());
            self.replies
                .lock()
                .unwrap()
                .pop_front()
                .ok_or_else(|| "fake http: no reply left".into())
        }
    }

    fn reply(status: u16, body: &str) -> HttpResponse {
        HttpResponse {
            status,
            body: body.to_owned(),
            retry_after: None,
        }
    }

    struct Harness {
        api: SpotifyApi,
        http: Arc<Fake>,
        sleeps: Arc<Mutex<Vec<Duration>>>,
        refreshes: Arc<Mutex<u32>>,
    }

    impl Harness {
        fn calls(&self) -> Vec<HttpRequest> {
            self.http.calls.lock().unwrap().clone()
        }
    }

    fn make(replies: Vec<HttpResponse>) -> Harness {
        let http = Arc::new(Fake {
            replies: Mutex::new(replies.into()),
            ..Fake::default()
        });
        let sleeps = Arc::new(Mutex::new(Vec::new()));
        let refreshes = Arc::new(Mutex::new(0));
        let r = refreshes.clone();
        let s = sleeps.clone();
        let tokens = TokenFns {
            get: || Ok("tok1".to_owned()),
            refresh: move || {
                *r.lock().unwrap() += 1;
                Ok("tok2".to_owned())
            },
        };
        let api =
            SpotifyApi::new(tokens, http.clone()).with_sleep(move |d| s.lock().unwrap().push(d));
        Harness {
            api,
            http,
            sleeps,
            refreshes,
        }
    }

    fn header<'a>(req: &'a HttpRequest, name: &str) -> Option<&'a str> {
        req.headers
            .iter()
            .find(|(k, _)| *k == name)
            .map(|(_, v)| v.as_str())
    }

    #[test]
    fn builds_the_url_sends_the_bearer_token_and_parses_json() {
        let h = make(vec![reply(200, r#"{"a":1}"#)]);
        let r = h
            .api
            .request(
                Method::Get,
                "/me/playlists",
                RequestOpts::default().query("limit", "50"),
            )
            .unwrap();
        assert_eq!(
            r,
            ApiResponse {
                status: 200,
                json: Some(json!({ "a": 1 }))
            }
        );
        let calls = h.calls();
        assert_eq!(
            calls[0].url,
            "https://api.spotify.com/v1/me/playlists?limit=50"
        );
        assert_eq!(calls[0].method, Method::Get);
        assert_eq!(header(&calls[0], "Authorization"), Some("Bearer tok1"));
    }

    #[test]
    fn serialises_the_body_as_json() {
        let h = make(vec![reply(200, "")]);
        h.api
            .request(
                Method::Put,
                "/me/player",
                RequestOpts::default().body(json!({ "device_ids": ["d"] })),
            )
            .unwrap();
        let calls = h.calls();
        assert_eq!(calls[0].body.as_deref(), Some(r#"{"device_ids":["d"]}"#));
        assert_eq!(header(&calls[0], "Content-Type"), Some("application/json"));
    }

    #[test]
    fn accepts_an_absolute_url_so_paging_can_follow_next() {
        let h = make(vec![reply(200, "{}")]);
        let next = "https://api.spotify.com/v1/me/playlists?offset=50&limit=50";
        h.api
            .request(Method::Get, next, RequestOpts::default())
            .unwrap();
        assert_eq!(h.calls()[0].url, next);
    }

    #[test]
    fn encodes_query_values_like_url_search_params() {
        assert_eq!(
            build_url(
                "/playlists/x",
                &[("fields".into(), "name,uri,images,owner.id".into())]
            ),
            "https://api.spotify.com/v1/playlists/x?fields=name%2Curi%2Cimages%2Cowner.id"
        );
        assert_eq!(
            build_url(
                "https://api.spotify.com/v1/a?limit=1&x=y&limit=2",
                &[("limit".into(), "50".into())]
            ),
            "https://api.spotify.com/v1/a?limit=50&x=y"
        );
    }

    #[test]
    fn gives_json_none_for_a_200_with_an_empty_body() {
        let h = make(vec![reply(200, "")]);
        let r = h
            .api
            .request(Method::Put, "/me/player/shuffle", RequestOpts::default())
            .unwrap();
        assert_eq!(
            r,
            ApiResponse {
                status: 200,
                json: None
            }
        );
    }

    #[test]
    fn gives_json_none_for_a_200_whose_body_is_not_json() {
        let h = make(vec![reply(200, "Hl7vflUgbZ")]);
        let r = h
            .api
            .request(Method::Put, "/me/player/repeat", RequestOpts::default())
            .unwrap();
        assert_eq!(
            r,
            ApiResponse {
                status: 200,
                json: None
            }
        );
    }

    #[test]
    fn resolves_any_2xx() {
        let h = make(vec![reply(204, "")]);
        let r = h
            .api
            .request(Method::Put, "/me/player/play", RequestOpts::default())
            .unwrap();
        assert_eq!(
            r,
            ApiResponse {
                status: 204,
                json: None
            }
        );
    }

    #[test]
    fn on_401_refreshes_once_and_retries_with_the_new_token() {
        let h = make(vec![reply(401, ""), reply(200, r#"{"ok":true}"#)]);
        let r = h
            .api
            .request(Method::Get, "/me", RequestOpts::default())
            .unwrap();
        assert_eq!(r.json, Some(json!({ "ok": true })));
        assert_eq!(*h.refreshes.lock().unwrap(), 1);
        let calls = h.calls();
        assert_eq!(header(&calls[0], "Authorization"), Some("Bearer tok1"));
        assert_eq!(header(&calls[1], "Authorization"), Some("Bearer tok2"));
    }

    #[test]
    fn on_a_second_401_returns_api_error() {
        let h = make(vec![reply(401, ""), reply(401, "")]);
        let err = h
            .api
            .request(Method::Get, "/me", RequestOpts::default())
            .unwrap_err();
        assert!(matches!(err, Error::Api(ApiError { status: 401, .. })));
        assert_eq!(*h.refreshes.lock().unwrap(), 1);
    }

    #[test]
    fn on_429_waits_retry_after_seconds_and_retries_once() {
        let h = make(vec![
            HttpResponse {
                status: 429,
                body: String::new(),
                retry_after: Some("3".into()),
            },
            reply(200, "{}"),
        ]);
        h.api
            .request(Method::Get, "/me", RequestOpts::default())
            .unwrap();
        assert_eq!(*h.sleeps.lock().unwrap(), vec![Duration::from_secs(3)]);
        assert_eq!(h.calls().len(), 2);
    }

    #[test]
    fn on_5xx_waits_1_s_retries_once_then_fails() {
        let h = make(vec![reply(502, ""), reply(503, r#"{"error":"x"}"#)]);
        let err = h
            .api
            .request(Method::Put, "/me/player/play", RequestOpts::default())
            .unwrap_err();
        let Error::Api(e) = err else {
            panic!("expected ApiError, got {err:?}")
        };
        assert_eq!(e.status, 503);
        assert_eq!(e.body, Some(json!({ "error": "x" })));
        assert_eq!(e.method, Method::Put);
        assert_eq!(e.path, "/me/player/play");
        assert_eq!(*h.sleeps.lock().unwrap(), vec![Duration::from_secs(1)]);
        assert_eq!(h.calls().len(), 2);
    }

    #[test]
    fn returns_api_error_on_a_404_without_retrying() {
        let h = make(vec![reply(404, "")]);
        let err = h
            .api
            .request(Method::Get, "/playlists/x", RequestOpts::default())
            .unwrap_err();
        assert_eq!(err.api_status(), Some(404));
        assert_eq!(h.calls().len(), 1);
    }

    #[test]
    fn logs_every_non_2xx_as_method_path_status() {
        let h = make(vec![reply(401, ""), reply(404, "")]);
        let logs = test_log::capture(|| {
            let _ = h.api.request(Method::Get, "/me", RequestOpts::default());
        });
        assert_eq!(logs, vec!["GET /me 401", "GET /me 404"]);
    }

    #[test]
    fn falls_back_to_1_s_for_a_missing_or_odd_retry_after() {
        assert_eq!(retry_after(None), Duration::from_secs(1));
        assert_eq!(retry_after(Some("soon")), Duration::from_secs(1));
        assert_eq!(retry_after(Some("0")), Duration::from_secs(1));
        assert_eq!(retry_after(Some("1.5")), Duration::from_millis(1500));
    }

    #[test]
    fn passes_transport_failures_through_without_retrying() {
        let h = make(vec![]);
        let err = h
            .api
            .request(Method::Get, "/me", RequestOpts::default())
            .unwrap_err();
        assert!(matches!(err, Error::Transport(_)));
        assert_eq!(h.calls().len(), 1);
    }
}
