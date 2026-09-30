use std::time::Duration;

use ureq::Agent;

use crate::api::{BoxError, HttpClient, HttpRequest, HttpResponse, Method};

/// The real `HttpClient`: blocking ureq over rustls.
pub struct UreqClient {
    agent: Agent,
}

impl UreqClient {
    pub fn new() -> Self {
        let config = Agent::config_builder()
            // The retry logic in SpotifyApi needs every status back as a response, not an error.
            .http_status_as_error(false)
            // fetch never gave up on its own; a background thread must, or a dead connection
            // holds it forever.
            .timeout_global(Some(Duration::from_secs(30)))
            .build();
        Self {
            agent: config.into(),
        }
    }
}

impl Default for UreqClient {
    fn default() -> Self {
        Self::new()
    }
}

impl HttpClient for UreqClient {
    fn send(&self, req: &HttpRequest) -> Result<HttpResponse, BoxError> {
        let mut builder = ureq::http::Request::builder()
            .method(req.method.as_str())
            .uri(&req.url);
        for (name, value) in &req.headers {
            builder = builder.header(*name, value);
        }
        let mut res = match (&req.body, req.method) {
            (Some(body), _) => self.agent.run(builder.body(body.as_bytes())?)?,
            // ureq refuses a body on GET, even an empty one.
            (None, Method::Get | Method::Delete) => self.agent.run(builder.body(())?)?,
            // fetch sent `Content-Length: 0` on a bodiless PUT; keep that rather than find out
            // which endpoints mind its absence.
            (None, Method::Put | Method::Post) => self.agent.run(builder.body(&[][..])?)?,
        };
        let retry_after = res
            .headers()
            .get("retry-after")
            .and_then(|v| v.to_str().ok())
            .map(str::to_owned);
        let status = res.status().as_u16();
        let body = res.body_mut().read_to_string()?;
        Ok(HttpResponse {
            status,
            body,
            retry_after,
        })
    }
}
