//! The HTTP client GPUI's `img()` fetches artwork with. GPUI ships none; this is the least one
//! can be: https GETs on a thread each, with the ureq the Web API client already uses.

use futures::channel::oneshot;
use futures::future::BoxFuture;
use gpui::http_client::http::HeaderValue;
use gpui::http_client::{AsyncBody, HttpClient, Method, Request, Response, Url};

const MAX_BYTES: u64 = 8 * 1024 * 1024;

pub struct ImageHttp {
    agent: ureq::Agent,
}

impl ImageHttp {
    pub fn new() -> Self {
        let config = ureq::Agent::config_builder()
            .timeout_global(Some(std::time::Duration::from_secs(30)))
            .http_status_as_error(false)
            .build();
        Self {
            agent: config.into(),
        }
    }
}

impl HttpClient for ImageHttp {
    fn user_agent(&self) -> Option<&HeaderValue> {
        None
    }

    fn proxy(&self) -> Option<&Url> {
        None
    }

    fn send(
        &self,
        req: Request<AsyncBody>,
    ) -> BoxFuture<'static, anyhow::Result<Response<AsyncBody>>> {
        let uri = req.uri().to_string();
        if req.method() != Method::GET || !uri.starts_with("https://") {
            return Box::pin(async move { Err(anyhow::anyhow!("only https GETs: {uri}")) });
        }
        let agent = self.agent.clone();
        let (tx, rx) = oneshot::channel();
        std::thread::spawn(move || {
            let _ = tx.send(fetch(&agent, &uri));
        });
        Box::pin(async move { rx.await? })
    }
}

fn fetch(agent: &ureq::Agent, uri: &str) -> anyhow::Result<Response<AsyncBody>> {
    let mut response = agent.get(uri).call()?;
    let status = response.status();
    let body = response
        .body_mut()
        .with_config()
        .limit(MAX_BYTES)
        .read_to_vec()?;
    Ok(Response::builder()
        .status(status.as_u16())
        .body(AsyncBody::from(body))?)
}
