// A `WebApi` that answers from a routing closure and records every call.

use std::sync::Mutex;

use serde_json::Value;

use crate::api::{ApiError, ApiResponse, Error, Method, RequestOpts, WebApi};

#[derive(Debug, Clone, PartialEq)]
pub struct Call {
    pub method: Method,
    pub path: String,
    pub query: Vec<(String, String)>,
    pub body: Option<Value>,
}

type Route = Box<dyn Fn(&Call) -> Result<ApiResponse, Error> + Send + Sync>;

pub struct FakeApi {
    route: Route,
    calls: Mutex<Vec<Call>>,
}

impl FakeApi {
    pub fn new(
        route: impl Fn(&Call) -> Result<ApiResponse, Error> + Send + Sync + 'static,
    ) -> Self {
        Self {
            route: Box::new(route),
            calls: Mutex::new(Vec::new()),
        }
    }

    /// Answers 200 with no body to everything.
    pub fn ok() -> Self {
        Self::new(|_| {
            Ok(ApiResponse {
                status: 200,
                json: None,
            })
        })
    }

    pub fn calls(&self) -> Vec<Call> {
        self.calls.lock().unwrap().clone()
    }
}

impl WebApi for FakeApi {
    fn request(&self, method: Method, path: &str, opts: RequestOpts) -> Result<ApiResponse, Error> {
        let call = Call {
            method,
            path: path.to_owned(),
            query: opts.query,
            body: opts.body,
        };
        self.calls.lock().unwrap().push(call.clone());
        (self.route)(&call)
    }
}

pub fn ok(json: Value) -> Result<ApiResponse, Error> {
    Ok(ApiResponse {
        status: 200,
        json: Some(json),
    })
}

pub fn api_error(call: &Call, status: u16) -> Result<ApiResponse, Error> {
    Err(Error::Api(ApiError {
        method: call.method,
        path: call.path.clone(),
        status,
        body: None,
    }))
}

pub fn query(pairs: &[(&str, &str)]) -> Vec<(String, String)> {
    pairs
        .iter()
        .map(|(k, v)| ((*k).to_owned(), (*v).to_owned()))
        .collect()
}
