use std::fmt;
use std::time::Duration;

/// The token store's only way out to the network, so tests can fake Spotify.
pub trait Http: Send + Sync {
    /// Any HTTP status is an `Ok`; `Err` means the request never got an answer.
    fn send(&self, request: HttpRequest) -> Result<HttpResponse, HttpError>;
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Method {
    Get,
    Post,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HttpRequest {
    pub method: Method,
    pub url: String,
    pub headers: Vec<(String, String)>,
    pub body: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HttpResponse {
    pub status: u16,
    pub body: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HttpError(pub String);

impl fmt::Display for HttpError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for HttpError {}

const TIMEOUT: Duration = Duration::from_secs(30);

/// Blocking HTTP through `ureq`. Call it from a background thread.
pub struct UreqHttp {
    agent: ureq::Agent,
}

impl UreqHttp {
    pub fn new() -> Self {
        let config = ureq::Agent::config_builder()
            .http_status_as_error(false)
            .timeout_global(Some(TIMEOUT))
            .build();
        Self {
            agent: config.into(),
        }
    }
}

impl Default for UreqHttp {
    fn default() -> Self {
        Self::new()
    }
}

impl Http for UreqHttp {
    fn send(&self, request: HttpRequest) -> Result<HttpResponse, HttpError> {
        let err = |e: ureq::Error| HttpError(e.to_string());
        let mut response = match request.method {
            Method::Get => {
                let mut req = self.agent.get(&request.url);
                for (k, v) in &request.headers {
                    req = req.header(k, v);
                }
                req.call().map_err(err)?
            }
            Method::Post => {
                let mut req = self.agent.post(&request.url);
                for (k, v) in &request.headers {
                    req = req.header(k, v);
                }
                req.send(request.body.unwrap_or_default()).map_err(err)?
            }
        };
        Ok(HttpResponse {
            status: response.status().as_u16(),
            body: response.body_mut().read_to_string().map_err(err)?,
        })
    }
}
