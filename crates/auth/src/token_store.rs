use std::fmt;
use std::process::Command;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError, TryLockError, Weak};
use std::time::{Duration, Instant};

use serde::Deserialize;

use crate::callback_server::{CallbackError, CallbackErrorCode, CallbackOptions, CallbackServer};
use crate::http::{Http, HttpRequest, Method};
use crate::keychain::{Keychain, KeychainError};
use crate::pkce::{CALLBACK_PORT, REDIRECT_URI, build_authorize_url, generate_pkce};
use crate::scheduler::{Scheduler, ThreadScheduler};

const TOKEN_URL: &str = "https://accounts.spotify.com/api/token";
const ME_URL: &str = "https://api.spotify.com/v1/me";
const REFRESH_LEAD: Duration = Duration::from_secs(5 * 60);
const BACKOFF: [Duration; 3] = [
    Duration::from_secs(2),
    Duration::from_secs(4),
    Duration::from_secs(8),
];
const BACKOFF_STEADY: Duration = Duration::from_secs(30);

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AuthState {
    SignedOut {
        reason: SignedOutReason,
        detail: Option<String>,
    },
    SigningIn,
    SignedIn {
        display_name: String,
        user_id: String,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SignedOutReason {
    FirstRun,
    Expired,
    NotPremium,
    Error,
}

impl AuthState {
    fn signed_out(reason: SignedOutReason) -> Self {
        Self::SignedOut {
            reason,
            detail: None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AuthError {
    SignedOut,
    NoRefreshToken,
    Callback(CallbackError),
    Browser(String),
    /// No answer from Spotify: the request failed, or the body made no sense.
    Network(String),
    TokenEndpoint {
        status: u16,
        code: Option<String>,
        message: String,
    },
    Profile(String),
    Keychain(KeychainError),
}

impl AuthError {
    fn is_invalid_grant(&self) -> bool {
        matches!(self, Self::TokenEndpoint { status: 400, code: Some(c), .. } if c == "invalid_grant")
    }
}

impl fmt::Display for AuthError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::SignedOut => f.write_str("Signed out"),
            Self::NoRefreshToken => f.write_str("No refresh token"),
            Self::Callback(e) => e.fmt(f),
            Self::Browser(m) => write!(f, "Could not open the browser: {m}"),
            Self::Network(m) | Self::Profile(m) | Self::TokenEndpoint { message: m, .. } => {
                f.write_str(m)
            }
            Self::Keychain(e) => e.fmt(f),
        }
    }
}

impl std::error::Error for AuthError {}

#[derive(Deserialize)]
struct TokenResponse {
    access_token: String,
    expires_in: u64,
    refresh_token: Option<String>,
}

#[derive(Deserialize)]
struct Me {
    id: String,
    display_name: Option<String>,
}

#[derive(Clone)]
struct Profile {
    user_id: String,
    display_name: String,
}

type Listener = Arc<dyn Fn(&AuthState) + Send + Sync>;
type OpenBrowser = Box<dyn Fn(&str) -> Result<(), String> + Send + Sync>;
type Listen = Box<dyn Fn(&str) -> Result<CallbackServer, CallbackError> + Send + Sync>;

/// Owns the Listener's tokens. Cheap to clone; every clone is the same store.
#[derive(Clone)]
pub struct TokenStore(Arc<Inner>);

struct Inner {
    client_id: String,
    keychain: Box<dyn Keychain>,
    http: Box<dyn Http>,
    open_browser: OpenBrowser,
    listen: Listen,
    scheduler: Arc<dyn Scheduler>,
    tokens: Mutex<Tokens>,
    listeners: Mutex<(u64, Vec<(u64, Listener)>)>,
    // Held for the length of one refresh, so concurrent callers share its result.
    refresh_lock: Mutex<()>,
    // Held for the length of one sign-in, so a second click waits for the first flow.
    flow_lock: Mutex<()>,
}

struct Tokens {
    state: AuthState,
    refresh_token: Option<String>,
    // What the Keychain holds, so a rotation whose write failed is retried on the next refresh.
    persisted: Option<String>,
    access_token: Option<String>,
    expires_at: Option<Instant>,
    profile: Option<Profile>,
    // Bumped to cancel the pending timer; a task that wakes to a different value does nothing.
    timer_gen: u64,
    failures: usize,
    refreshes: u64,
    last_refresh: Option<Result<String, AuthError>>,
}

fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(PoisonError::into_inner)
}

fn open_in_browser(url: &str) -> Result<(), String> {
    let status = Command::new("/usr/bin/open")
        .arg(url)
        .status()
        .map_err(|e| e.to_string())?;
    if status.success() {
        Ok(())
    } else {
        Err(format!("open exited with {status}"))
    }
}

impl TokenStore {
    /// `client_id` is the Spotify developer app this build signs in through.
    pub fn new(
        client_id: impl Into<String>,
        keychain: impl Keychain + 'static,
        http: impl Http + 'static,
    ) -> Self {
        Self::from_parts(
            client_id.into(),
            Box::new(keychain),
            Box::new(http),
            Box::new(open_in_browser),
            Box::new(|state| CallbackServer::start(state, CallbackOptions::default())),
            Arc::new(ThreadScheduler::new()),
        )
    }

    fn from_parts(
        client_id: String,
        keychain: Box<dyn Keychain>,
        http: Box<dyn Http>,
        open_browser: OpenBrowser,
        listen: Listen,
        scheduler: Arc<dyn Scheduler>,
    ) -> Self {
        Self(Arc::new(Inner {
            client_id,
            keychain,
            http,
            open_browser,
            listen,
            scheduler,
            tokens: Mutex::new(Tokens {
                state: AuthState::signed_out(SignedOutReason::FirstRun),
                refresh_token: None,
                persisted: None,
                access_token: None,
                expires_at: None,
                profile: None,
                timer_gen: 0,
                failures: 0,
                refreshes: 0,
                last_refresh: None,
            }),
            listeners: Mutex::new((0, Vec::new())),
            refresh_lock: Mutex::new(()),
            flow_lock: Mutex::new(()),
        }))
    }

    pub fn state(&self) -> AuthState {
        self.tokens().state.clone()
    }

    pub fn user_id(&self) -> Option<String> {
        self.tokens().profile.as_ref().map(|p| p.user_id.clone())
    }

    /// Calls `f` on every state change, on whichever thread made the change, until the
    /// subscription is dropped.
    pub fn on_state(&self, f: impl Fn(&AuthState) + Send + Sync + 'static) -> Subscription {
        let mut listeners = lock(&self.0.listeners);
        listeners.0 += 1;
        let id = listeners.0;
        listeners.1.push((id, Arc::new(f)));
        Subscription {
            store: Arc::downgrade(&self.0),
            id: Some(id),
        }
    }

    /// Picks up a stored sign-in, if there is one. Fails only when the Keychain can't be read.
    pub fn start(&self) -> Result<(), AuthError> {
        let Some(stored) = self.0.keychain.read().map_err(AuthError::Keychain)? else {
            self.publish(AuthState::signed_out(SignedOutReason::FirstRun));
            return Ok(());
        };
        {
            let mut t = self.tokens();
            t.refresh_token = Some(stored.clone());
            t.persisted = Some(stored);
        }
        self.publish(AuthState::SigningIn);
        let _ = self.refresh();
        Ok(())
    }

    /// Runs the browser flow and returns when it has landed one way or the other. The outcome is
    /// published as state, never returned.
    pub fn sign_in(&self) {
        let _flow = match self.0.flow_lock.try_lock() {
            Ok(guard) => guard,
            Err(TryLockError::Poisoned(p)) => p.into_inner(),
            Err(TryLockError::WouldBlock) => {
                drop(lock(&self.0.flow_lock));
                return;
            }
        };
        self.publish(AuthState::SigningIn);
        self.tokens().profile = None;
        if let Err(err) = self.run_pkce_flow() {
            log::error!("sign-in failed: {err}");
            self.publish(AuthState::SignedOut {
                reason: SignedOutReason::Error,
                detail: Some(describe_sign_in_error(&err)),
            });
        }
    }

    /// A token with more than five minutes left, refreshing first if needed. If the refresh
    /// fails, a cached token that hasn't expired yet still counts.
    pub fn access_token(&self) -> Result<String, AuthError> {
        {
            let t = self.tokens();
            if matches!(t.state, AuthState::SignedOut { .. }) {
                return Err(AuthError::SignedOut);
            }
            if let Some(token) = &t.access_token
                && self.time_left(&t) > REFRESH_LEAD
            {
                return Ok(token.clone());
            }
        }
        self.refresh().or_else(|err| {
            let t = self.tokens();
            match &t.access_token {
                Some(token) if self.time_left(&t) > Duration::ZERO => Ok(token.clone()),
                _ => Err(err),
            }
        })
    }

    /// Refreshes now, whatever the cached token's age. For a 401 from the Web API.
    pub fn force_refresh(&self) -> Result<String, AuthError> {
        self.refresh()
    }

    /// The Player's `account_error`: the refresh token stays, since the account itself is fine.
    pub fn report_not_premium(&self) {
        self.publish(AuthState::signed_out(SignedOutReason::NotPremium));
    }

    fn run_pkce_flow(&self) -> Result<(), AuthError> {
        let pkce = generate_pkce();
        let code = {
            let server = (self.0.listen)(&pkce.state).map_err(AuthError::Callback)?;
            (self.0.open_browser)(&build_authorize_url(
                &self.0.client_id,
                &pkce.challenge,
                &pkce.state,
            ))
            .map_err(AuthError::Browser)?;
            server.wait().map_err(AuthError::Callback)?
        };
        let tokens = self.post_token(&[
            ("grant_type", "authorization_code"),
            ("code", &code),
            ("redirect_uri", REDIRECT_URI),
            ("client_id", &self.0.client_id),
            ("code_verifier", &pkce.verifier),
        ])?;
        self.adopt(tokens).map(drop)
    }

    fn refresh(&self) -> Result<String, AuthError> {
        let seen = self.tokens().refreshes;
        let _guard = lock(&self.0.refresh_lock);
        {
            let t = self.tokens();
            if t.refreshes != seen
                && let Some(result) = &t.last_refresh
            {
                return result.clone();
            }
        }
        let result = self.refresh_once();
        let mut t = self.tokens();
        t.refreshes += 1;
        t.last_refresh = Some(result.clone());
        result
    }

    fn refresh_once(&self) -> Result<String, AuthError> {
        let refresh_token = self
            .tokens()
            .refresh_token
            .clone()
            .ok_or(AuthError::NoRefreshToken)?;
        self.clear_timer();
        let result = self
            .post_token(&[
                ("grant_type", "refresh_token"),
                ("refresh_token", &refresh_token),
                ("client_id", &self.0.client_id),
            ])
            .and_then(|tokens| self.adopt(tokens));
        match &result {
            Ok(_) => self.tokens().failures = 0,
            Err(err) if err.is_invalid_grant() => self.expire(),
            Err(err) => self.schedule_retry(err),
        }
        result
    }

    fn adopt(&self, tokens: TokenResponse) -> Result<String, AuthError> {
        let now = self.0.scheduler.now();
        let profile = {
            let mut t = self.tokens();
            t.access_token = Some(tokens.access_token.clone());
            t.expires_at = Some(now + Duration::from_secs(tokens.expires_in));
            if let Some(next) = tokens.refresh_token {
                t.refresh_token = Some(next);
            }
            t.profile.clone()
        };
        let profile = match profile {
            Some(p) => p,
            None => {
                let p = self.fetch_profile(&tokens.access_token)?;
                self.tokens().profile = Some(p.clone());
                p
            }
        };
        let unsaved = {
            let t = self.tokens();
            t.refresh_token
                .clone()
                .filter(|rt| t.persisted.as_ref() != Some(rt))
        };
        if let Some(rt) = unsaved {
            self.0
                .keychain
                .write(&profile.user_id, &rt)
                .map_err(AuthError::Keychain)?;
            self.tokens().persisted = Some(rt);
        }
        self.schedule_refresh();
        if self.tokens().state == AuthState::SigningIn {
            self.publish(AuthState::SignedIn {
                display_name: profile.display_name,
                user_id: profile.user_id,
            });
        }
        Ok(tokens.access_token)
    }

    fn expire(&self) {
        log::warn!("refresh token rejected with invalid_grant, signing out");
        {
            let mut t = self.tokens();
            t.refresh_token = None;
            t.persisted = None;
            t.access_token = None;
            t.profile = None;
        }
        self.clear_timer();
        if let Err(err) = self.0.keychain.delete() {
            log::error!("could not delete the dead refresh token: {err}");
        }
        self.publish(AuthState::signed_out(SignedOutReason::Expired));
    }

    fn schedule_refresh(&self) {
        let delay = {
            let t = self.tokens();
            self.time_left(&t).saturating_sub(REFRESH_LEAD)
        };
        self.set_timer(delay);
    }

    fn schedule_retry(&self, err: &AuthError) {
        let delay = {
            let mut t = self.tokens();
            let delay = BACKOFF.get(t.failures).copied().unwrap_or(BACKOFF_STEADY);
            t.failures += 1;
            delay
        };
        log::warn!(
            "token refresh failed, retrying in {} ms: {err}",
            delay.as_millis()
        );
        self.set_timer(delay);
    }

    fn set_timer(&self, delay: Duration) {
        let generation = {
            let mut t = self.tokens();
            t.timer_gen += 1;
            t.timer_gen
        };
        let store = Arc::downgrade(&self.0);
        self.0.scheduler.after(
            delay,
            Box::new(move || {
                let Some(inner) = store.upgrade() else { return };
                let store = TokenStore(inner);
                if store.tokens().timer_gen == generation {
                    let _ = store.refresh();
                }
            }),
        );
    }

    fn clear_timer(&self) {
        self.tokens().timer_gen += 1;
    }

    fn post_token(&self, form: &[(&str, &str)]) -> Result<TokenResponse, AuthError> {
        let body = form_urlencoded::Serializer::new(String::new())
            .extend_pairs(form)
            .finish();
        let res = self
            .0
            .http
            .send(HttpRequest {
                method: Method::Post,
                url: TOKEN_URL.into(),
                headers: vec![(
                    "content-type".into(),
                    "application/x-www-form-urlencoded".into(),
                )],
                body: Some(body),
            })
            .map_err(|e| AuthError::Network(e.0))?;
        if !(200..300).contains(&res.status) {
            let body: serde_json::Value = serde_json::from_str(&res.body).unwrap_or_default();
            let code = body["error"].as_str().map(str::to_string);
            let detail = body["error_description"]
                .as_str()
                .or(code.as_deref())
                .unwrap_or("");
            log::warn!("POST /api/token {}: {detail}", res.status);
            return Err(AuthError::TokenEndpoint {
                status: res.status,
                message: format!("Token endpoint {}: {detail}", res.status)
                    .trim()
                    .to_string(),
                code,
            });
        }
        // serde's message names the missing field, never a value, so it is safe to log.
        serde_json::from_str(&res.body)
            .map_err(|e| AuthError::Network(format!("Token endpoint sent an odd body: {e}")))
    }

    fn fetch_profile(&self, access_token: &str) -> Result<Profile, AuthError> {
        let res = self
            .0
            .http
            .send(HttpRequest {
                method: Method::Get,
                url: ME_URL.into(),
                headers: vec![("authorization".into(), format!("Bearer {access_token}"))],
                body: None,
            })
            .map_err(|e| AuthError::Network(e.0))?;
        if !(200..300).contains(&res.status) {
            log::warn!("GET /v1/me {}", res.status);
            return Err(AuthError::Profile(format!("GET /v1/me {}", res.status)));
        }
        let me: Me = serde_json::from_str(&res.body)
            .map_err(|e| AuthError::Profile(format!("GET /v1/me sent an odd body: {e}")))?;
        Ok(Profile {
            display_name: me.display_name.unwrap_or_else(|| me.id.clone()),
            user_id: me.id,
        })
    }

    fn tokens(&self) -> MutexGuard<'_, Tokens> {
        lock(&self.0.tokens)
    }

    fn time_left(&self, t: &Tokens) -> Duration {
        t.expires_at.map_or(Duration::ZERO, |at| {
            at.saturating_duration_since(self.0.scheduler.now())
        })
    }

    fn publish(&self, next: AuthState) {
        self.tokens().state = next.clone();
        // Call outside the lock so a listener may read the store or subscribe.
        let listeners: Vec<Listener> = lock(&self.0.listeners)
            .1
            .iter()
            .map(|(_, f)| f.clone())
            .collect();
        for f in listeners {
            f(&next);
        }
    }
}

fn describe_sign_in_error(err: &AuthError) -> String {
    match err {
        AuthError::Callback(e) if e.code == CallbackErrorCode::PortInUse => {
            format!("Port {CALLBACK_PORT} is in use")
        }
        other => other.to_string(),
    }
}

/// Stops the `on_state` callback when dropped.
#[must_use = "dropping a Subscription unsubscribes; call detach() to keep it"]
pub struct Subscription {
    store: Weak<Inner>,
    id: Option<u64>,
}

impl Subscription {
    /// Keeps the callback for the store's lifetime.
    pub fn detach(mut self) {
        self.id = None;
    }
}

impl Drop for Subscription {
    fn drop(&mut self) {
        if let (Some(id), Some(inner)) = (self.id, self.store.upgrade()) {
            lock(&inner.listeners).1.retain(|(i, _)| *i != id);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::http::{HttpError, HttpResponse};
    use crate::scheduler::fake::FakeScheduler;
    use serde_json::{Value, json};
    use std::collections::{HashMap, VecDeque};
    use std::sync::Barrier;
    use std::thread;

    const MIN: Duration = Duration::from_secs(60);
    const MS: Duration = Duration::from_millis(1);
    const CLIENT_ID: &str = "client-1";

    #[derive(Clone)]
    enum Reply {
        Status(u16, Value),
        Fail(&'static str),
    }

    struct Call {
        url: String,
        method: Method,
        headers: HashMap<String, String>,
        body: HashMap<String, String>,
    }

    #[derive(Default)]
    struct FakeHttp {
        token: Mutex<VecDeque<Reply>>,
        me: Mutex<VecDeque<Reply>>,
        calls: Mutex<Vec<Call>>,
        token_delay: Option<Duration>,
    }

    impl Http for FakeHttp {
        fn send(&self, req: HttpRequest) -> Result<HttpResponse, HttpError> {
            let body = req
                .body
                .as_deref()
                .map(|b| form_urlencoded::parse(b.as_bytes()).into_owned().collect())
                .unwrap_or_default();
            let is_token = req.url == TOKEN_URL;
            self.calls.lock().unwrap().push(Call {
                url: req.url,
                method: req.method,
                headers: req.headers.into_iter().collect(),
                body,
            });
            if is_token && let Some(d) = self.token_delay {
                thread::sleep(d);
            }
            // Like the TS fake: replies are used in order, and the last one repeats forever.
            let mut queue = if is_token { &self.token } else { &self.me }
                .lock()
                .unwrap();
            let reply = if queue.len() > 1 {
                queue.pop_front()
            } else {
                queue.front().cloned()
            };
            match reply.expect("no reply queued") {
                Reply::Status(status, body) => Ok(HttpResponse {
                    status,
                    body: body.to_string(),
                }),
                Reply::Fail(m) => Err(HttpError(m.into())),
            }
        }
    }

    impl FakeHttp {
        fn token_calls(&self) -> Vec<HashMap<String, String>> {
            let calls = self.calls.lock().unwrap();
            calls
                .iter()
                .filter(|c| c.url == TOKEN_URL)
                .map(|c| c.body.clone())
                .collect()
        }
    }

    #[derive(Default)]
    struct FakeKeychain {
        stored: Mutex<Option<String>>,
        writes: Mutex<Vec<(String, String)>>,
        deletes: Mutex<usize>,
        fail_writes: Mutex<usize>,
    }

    impl Keychain for FakeKeychain {
        fn read(&self) -> Result<Option<String>, KeychainError> {
            Ok(self.stored.lock().unwrap().clone())
        }
        fn write(&self, user_id: &str, token: &str) -> Result<(), KeychainError> {
            let mut fail = self.fail_writes.lock().unwrap();
            if *fail > 0 {
                *fail -= 1;
                return Err(KeychainError("locked".into()));
            }
            self.writes
                .lock()
                .unwrap()
                .push((user_id.into(), token.into()));
            *self.stored.lock().unwrap() = Some(token.into());
            Ok(())
        }
        fn delete(&self) -> Result<(), KeychainError> {
            *self.deletes.lock().unwrap() += 1;
            *self.stored.lock().unwrap() = None;
            Ok(())
        }
    }

    impl FakeKeychain {
        fn writes(&self) -> Vec<(String, String)> {
            self.writes.lock().unwrap().clone()
        }
        fn deletes(&self) -> usize {
            *self.deletes.lock().unwrap()
        }
    }

    // Box<dyn Trait> needs owned values; these let the test keep a handle on the same fake.
    impl Http for Arc<FakeHttp> {
        fn send(&self, req: HttpRequest) -> Result<HttpResponse, HttpError> {
            (**self).send(req)
        }
    }
    impl Keychain for Arc<FakeKeychain> {
        fn read(&self) -> Result<Option<String>, KeychainError> {
            (**self).read()
        }
        fn write(&self, user_id: &str, token: &str) -> Result<(), KeychainError> {
            (**self).write(user_id, token)
        }
        fn delete(&self) -> Result<(), KeychainError> {
            (**self).delete()
        }
    }

    fn tokens_with(over: Value) -> Reply {
        let mut body =
            json!({ "access_token": "at-1", "token_type": "Bearer", "expires_in": 3600 });
        body.as_object_mut()
            .unwrap()
            .extend(over.as_object().unwrap().clone());
        Reply::Status(200, body)
    }

    fn tokens() -> Reply {
        tokens_with(json!({}))
    }

    fn me() -> Reply {
        Reply::Status(200, json!({ "id": "user-1", "display_name": "Jonas" }))
    }

    fn invalid_grant(description: &str) -> Reply {
        Reply::Status(
            400,
            json!({ "error": "invalid_grant", "error_description": description }),
        )
    }

    fn signed_in(display_name: &str, user_id: &str) -> AuthState {
        AuthState::SignedIn {
            display_name: display_name.into(),
            user_id: user_id.into(),
        }
    }

    type Callback = Box<dyn Fn() -> Result<CallbackServer, CallbackError> + Send + Sync>;

    struct Build {
        stored: Option<&'static str>,
        token: Vec<Reply>,
        me: Vec<Reply>,
        callback: Option<Callback>,
        token_delay: Option<Duration>,
    }

    impl Default for Build {
        fn default() -> Self {
            Self {
                stored: Some("rt-0"),
                token: vec![tokens()],
                me: vec![me()],
                callback: None,
                token_delay: None,
            }
        }
    }

    struct Harness {
        store: TokenStore,
        keychain: Arc<FakeKeychain>,
        http: Arc<FakeHttp>,
        clock: Arc<FakeScheduler>,
        states: Arc<Mutex<Vec<AuthState>>>,
        opened: Arc<Mutex<Vec<String>>>,
        listened: Arc<Mutex<Vec<String>>>,
    }

    impl Harness {
        fn states(&self) -> Vec<AuthState> {
            self.states.lock().unwrap().clone()
        }
        fn last_state(&self) -> AuthState {
            self.states().last().cloned().unwrap()
        }
    }

    fn build(b: Build) -> Harness {
        let keychain = Arc::new(FakeKeychain {
            stored: Mutex::new(b.stored.map(str::to_string)),
            ..Default::default()
        });
        let http = Arc::new(FakeHttp {
            token: Mutex::new(b.token.into()),
            me: Mutex::new(b.me.into()),
            token_delay: b.token_delay,
            ..Default::default()
        });
        let clock = Arc::new(FakeScheduler::new());
        let opened = Arc::new(Mutex::new(Vec::new()));
        let listened = Arc::new(Mutex::new(Vec::new()));
        let callback = b.callback.unwrap_or_else(|| {
            Box::new(|| {
                Err(CallbackError::new(
                    CallbackErrorCode::Io,
                    "no fake callback",
                ))
            })
        });
        let store = TokenStore::from_parts(
            CLIENT_ID.into(),
            Box::new(keychain.clone()),
            Box::new(http.clone()),
            Box::new({
                let opened = opened.clone();
                move |url| {
                    opened.lock().unwrap().push(url.to_string());
                    Ok(())
                }
            }),
            Box::new({
                let listened = listened.clone();
                move |state| {
                    listened.lock().unwrap().push(state.to_string());
                    callback()
                }
            }),
            clock.clone(),
        );
        let states = Arc::new(Mutex::new(Vec::new()));
        let seen = states.clone();
        store
            .on_state(move |s| seen.lock().unwrap().push(s.clone()))
            .detach();
        Harness {
            store,
            keychain,
            http,
            clock,
            states,
            opened,
            listened,
        }
    }

    fn callback(outcome: Result<&'static str, CallbackError>) -> Option<Callback> {
        Some(Box::new(move || {
            Ok(CallbackServer::settled(outcome.clone().map(str::to_string)))
        }))
    }

    fn form(pairs: &[(&str, &str)]) -> HashMap<String, String> {
        pairs
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect()
    }

    // start

    #[test]
    fn start_publishes_first_run_when_the_keychain_is_empty() {
        let h = build(Build {
            stored: None,
            ..Default::default()
        });
        h.store.start().unwrap();
        let first_run = AuthState::signed_out(SignedOutReason::FirstRun);
        assert_eq!(h.store.state(), first_run);
        assert_eq!(h.states(), [first_run]);
        assert!(h.http.calls.lock().unwrap().is_empty());
    }

    #[test]
    fn start_refreshes_with_the_stored_token_loads_the_profile_and_publishes_signed_in() {
        let h = build(Build::default());
        h.store.start().unwrap();
        let calls = h.http.calls.lock().unwrap();
        let (refresh, profile) = (&calls[0], &calls[1]);
        assert_eq!(refresh.method, Method::Post);
        assert_eq!(
            refresh.headers["content-type"],
            "application/x-www-form-urlencoded"
        );
        assert_eq!(
            refresh.body,
            form(&[
                ("grant_type", "refresh_token"),
                ("refresh_token", "rt-0"),
                ("client_id", CLIENT_ID),
            ])
        );
        assert_eq!(profile.url, ME_URL);
        assert_eq!(profile.headers["authorization"], "Bearer at-1");
        assert_eq!(h.store.state(), signed_in("Jonas", "user-1"));
        assert_eq!(h.store.user_id().as_deref(), Some("user-1"));
    }

    #[test]
    fn start_falls_back_to_the_user_id_when_there_is_no_display_name() {
        let h = build(Build {
            me: vec![Reply::Status(
                200,
                json!({ "id": "user-1", "display_name": null }),
            )],
            ..Default::default()
        });
        h.store.start().unwrap();
        assert_eq!(h.store.state(), signed_in("user-1", "user-1"));
    }

    // scheduled refresh

    #[test]
    fn scheduled_refresh_runs_five_minutes_before_expiry() {
        let h = build(Build {
            token: vec![tokens(), tokens_with(json!({ "access_token": "at-2" }))],
            ..Default::default()
        });
        h.store.start().unwrap();
        assert_eq!(h.http.token_calls().len(), 1);
        h.clock.advance(55 * MIN - MS);
        assert_eq!(h.http.token_calls().len(), 1);
        h.clock.advance(MS);
        assert_eq!(h.http.token_calls().len(), 2);
        assert_eq!(h.store.access_token(), Ok("at-2".into()));
    }

    #[test]
    fn rotates_the_refresh_token_when_the_response_carries_one() {
        let h = build(Build {
            token: vec![tokens_with(json!({ "refresh_token": "rt-1" })), tokens()],
            ..Default::default()
        });
        h.store.start().unwrap();
        assert_eq!(h.keychain.writes(), [("user-1".into(), "rt-1".into())]);
        h.clock.advance(55 * MIN);
        assert_eq!(h.http.token_calls()[1]["refresh_token"], "rt-1");
    }

    #[test]
    fn retries_persisting_a_rotated_refresh_token_whose_write_failed() {
        let h = build(Build {
            token: vec![tokens_with(json!({ "refresh_token": "rt-1" })), tokens()],
            ..Default::default()
        });
        *h.keychain.fail_writes.lock().unwrap() = 1;
        h.store.start().unwrap();
        assert!(h.keychain.writes().is_empty());
        h.clock.advance(2 * BACKOFF[0]);
        assert_eq!(h.http.token_calls()[1]["refresh_token"], "rt-1");
        assert_eq!(h.keychain.writes(), [("user-1".into(), "rt-1".into())]);
    }

    #[test]
    fn does_not_touch_the_keychain_when_the_refresh_token_is_unchanged() {
        let h = build(Build::default());
        h.store.start().unwrap();
        assert!(h.keychain.writes().is_empty());
    }

    #[test]
    fn deletes_the_keychain_item_and_publishes_expired_on_invalid_grant() {
        let h = build(Build {
            token: vec![tokens(), invalid_grant("Refresh token revoked")],
            ..Default::default()
        });
        h.store.start().unwrap();
        h.clock.advance(55 * MIN);
        assert_eq!(h.keychain.deletes(), 1);
        assert_eq!(
            h.last_state(),
            AuthState::signed_out(SignedOutReason::Expired)
        );
        assert!(h.store.access_token().is_err());
    }

    #[test]
    fn backs_off_2s_4s_8s_then_every_30s_on_network_errors_and_stays_signed_in() {
        let h = build(Build {
            token: vec![tokens(), Reply::Fail("ECONNRESET")],
            ..Default::default()
        });
        h.store.start().unwrap();
        h.clock.advance(55 * MIN);
        assert_eq!(h.http.token_calls().len(), 2);
        for (secs, count) in [(2, 3), (4, 4), (8, 5), (30, 6), (30, 7)] {
            h.clock.advance(Duration::from_secs(secs) - MS);
            assert_eq!(h.http.token_calls().len(), count - 1);
            h.clock.advance(MS);
            assert_eq!(h.http.token_calls().len(), count);
        }
        assert!(matches!(h.store.state(), AuthState::SignedIn { .. }));
        assert!(
            !h.states()
                .iter()
                .any(|s| matches!(s, AuthState::SignedOut { .. }))
        );
        assert_eq!(h.keychain.deletes(), 0);
    }

    #[test]
    fn treats_5xx_like_a_network_error() {
        let h = build(Build {
            token: vec![tokens(), Reply::Status(503, json!({}))],
            ..Default::default()
        });
        h.store.start().unwrap();
        h.clock.advance(55 * MIN);
        h.clock.advance(Duration::from_secs(2));
        assert_eq!(h.http.token_calls().len(), 3);
        assert!(matches!(h.store.state(), AuthState::SignedIn { .. }));
    }

    #[test]
    fn resets_the_backoff_and_reschedules_once_a_retry_succeeds() {
        let h = build(Build {
            token: vec![
                tokens(),
                Reply::Fail("down"),
                tokens_with(json!({ "access_token": "at-3" })),
            ],
            ..Default::default()
        });
        h.store.start().unwrap();
        h.clock.advance(55 * MIN + Duration::from_secs(2));
        assert_eq!(h.http.token_calls().len(), 3);
        assert_eq!(h.store.access_token(), Ok("at-3".into()));
        h.clock.advance(55 * MIN);
        assert_eq!(h.http.token_calls().len(), 4);
    }

    // access_token

    #[test]
    fn returns_the_cached_token_while_more_than_five_minutes_remain() {
        let h = build(Build::default());
        h.store.start().unwrap();
        h.clock.set_elapsed(54 * MIN);
        assert_eq!(h.store.access_token(), Ok("at-1".into()));
        assert_eq!(h.http.token_calls().len(), 1);
    }

    #[test]
    fn refreshes_first_when_five_minutes_or_less_remain() {
        let h = build(Build {
            token: vec![tokens(), tokens_with(json!({ "access_token": "at-2" }))],
            ..Default::default()
        });
        h.store.start().unwrap();
        h.clock.set_elapsed(56 * MIN);
        assert_eq!(h.store.access_token(), Ok("at-2".into()));
        assert_eq!(h.http.token_calls().len(), 2);
    }

    #[test]
    fn falls_back_to_the_unexpired_cached_token_when_the_refresh_fails() {
        let h = build(Build {
            token: vec![tokens(), Reply::Fail("down")],
            ..Default::default()
        });
        h.store.start().unwrap();
        h.clock.set_elapsed(56 * MIN);
        assert_eq!(h.store.access_token(), Ok("at-1".into()));
    }

    #[test]
    fn rejects_while_signed_out() {
        let h = build(Build {
            stored: None,
            ..Default::default()
        });
        h.store.start().unwrap();
        let err = h.store.access_token().unwrap_err();
        assert_eq!(err, AuthError::SignedOut);
        assert!(err.to_string().to_lowercase().contains("signed out"));
    }

    #[test]
    fn shares_one_in_flight_refresh_between_callers() {
        let h = build(Build {
            token: vec![tokens(), tokens_with(json!({ "access_token": "at-2" }))],
            token_delay: Some(Duration::from_millis(100)),
            ..Default::default()
        });
        h.store.start().unwrap();
        h.clock.set_elapsed(56 * MIN);
        let gate = Arc::new(Barrier::new(2));
        let both: Vec<_> = (0..2)
            .map(|_| {
                let (store, gate) = (h.store.clone(), gate.clone());
                thread::spawn(move || {
                    gate.wait();
                    store.access_token()
                })
            })
            .collect::<Vec<_>>()
            .into_iter()
            .map(|t| t.join().unwrap())
            .collect();
        assert_eq!(both, [Ok("at-2".into()), Ok("at-2".into())]);
        assert_eq!(h.http.token_calls().len(), 2);
    }

    // force_refresh

    #[test]
    fn force_refresh_refreshes_even_when_the_cached_token_is_fresh() {
        let h = build(Build {
            token: vec![tokens(), tokens_with(json!({ "access_token": "at-2" }))],
            ..Default::default()
        });
        h.store.start().unwrap();
        assert_eq!(h.store.force_refresh(), Ok("at-2".into()));
        assert_eq!(h.http.token_calls().len(), 2);
    }

    // sign_in

    #[test]
    fn sign_in_opens_the_browser_exchanges_the_code_stores_the_refresh_token_and_publishes_signed_in()
     {
        let h = build(Build {
            stored: None,
            token: vec![tokens_with(json!({ "refresh_token": "rt-1" }))],
            callback: callback(Ok("code-1")),
            ..Default::default()
        });
        h.store.start().unwrap();
        h.store.sign_in();

        let kinds: Vec<_> = h.states().iter().map(std::mem::discriminant).collect();
        assert_eq!(
            kinds,
            [
                std::mem::discriminant(&AuthState::signed_out(SignedOutReason::FirstRun)),
                std::mem::discriminant(&AuthState::SigningIn),
                std::mem::discriminant(&signed_in("", "")),
            ]
        );
        let opened = h.opened.lock().unwrap()[0].clone();
        let (base, query) = opened.split_once('?').unwrap();
        assert_eq!(base, "https://accounts.spotify.com/authorize");
        let query: HashMap<String, String> = form_urlencoded::parse(query.as_bytes())
            .into_owned()
            .collect();
        assert_eq!(*h.listened.lock().unwrap(), [query["state"].clone()]);

        let exchange = h.http.token_calls()[0].clone();
        let verifier = exchange["code_verifier"].clone();
        assert!(verifier.len() >= 80);
        assert!(
            verifier
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
        );
        assert_eq!(
            exchange,
            form(&[
                ("grant_type", "authorization_code"),
                ("code", "code-1"),
                ("client_id", CLIENT_ID),
                ("redirect_uri", REDIRECT_URI),
                ("code_verifier", &verifier),
            ])
        );
        assert_eq!(h.keychain.writes(), [("user-1".into(), "rt-1".into())]);
        assert_eq!(h.store.state(), signed_in("Jonas", "user-1"));
        assert_eq!(h.store.access_token(), Ok("at-1".into()));
    }

    #[test]
    fn schedules_the_refresh_after_sign_in() {
        let h = build(Build {
            stored: None,
            token: vec![
                tokens_with(json!({ "refresh_token": "rt-1" })),
                tokens_with(json!({ "access_token": "at-2" })),
            ],
            callback: callback(Ok("code-1")),
            ..Default::default()
        });
        h.store.start().unwrap();
        h.store.sign_in();
        h.clock.advance(55 * MIN);
        assert_eq!(h.http.token_calls().len(), 2);
        assert_eq!(h.http.token_calls()[1]["refresh_token"], "rt-1");
    }

    #[test]
    fn publishes_error_when_the_callback_state_does_not_match() {
        let h = build(Build {
            stored: None,
            callback: callback(Err(CallbackError::new(
                CallbackErrorCode::StateMismatch,
                "Callback state did not match",
            ))),
            ..Default::default()
        });
        h.store.start().unwrap();
        h.store.sign_in();
        assert_eq!(
            h.last_state(),
            AuthState::SignedOut {
                reason: SignedOutReason::Error,
                detail: Some("Callback state did not match".into()),
            }
        );
        assert!(h.keychain.writes().is_empty());
    }

    #[test]
    fn reports_a_busy_port_in_the_spec_wording() {
        let h = build(Build {
            stored: None,
            callback: Some(Box::new(|| {
                Err(CallbackError::new(
                    CallbackErrorCode::PortInUse,
                    "listen EADDRINUSE",
                ))
            })),
            ..Default::default()
        });
        h.store.start().unwrap();
        h.store.sign_in();
        assert_eq!(
            h.last_state(),
            AuthState::SignedOut {
                reason: SignedOutReason::Error,
                detail: Some("Port 8888 is in use".into()),
            }
        );
        assert!(h.opened.lock().unwrap().is_empty());
    }

    #[test]
    fn fetches_the_profile_again_after_expired_so_a_different_listener_can_sign_in() {
        let h = build(Build {
            token: vec![
                tokens(),
                invalid_grant("Refresh token revoked"),
                tokens_with(json!({ "refresh_token": "rt-2" })),
            ],
            me: vec![
                me(),
                Reply::Status(200, json!({ "id": "user-2", "display_name": "Sam" })),
            ],
            callback: callback(Ok("code-1")),
            ..Default::default()
        });
        h.store.start().unwrap();
        h.clock.advance(55 * MIN);
        assert_eq!(
            h.last_state(),
            AuthState::signed_out(SignedOutReason::Expired)
        );
        h.store.sign_in();
        assert_eq!(h.store.state(), signed_in("Sam", "user-2"));
        assert_eq!(
            h.keychain.writes().last(),
            Some(&("user-2".into(), "rt-2".into()))
        );
    }

    #[test]
    fn publishes_error_with_the_message_when_the_exchange_fails() {
        let h = build(Build {
            stored: None,
            token: vec![invalid_grant("Invalid authorization code")],
            callback: callback(Ok("code-1")),
            ..Default::default()
        });
        h.store.start().unwrap();
        h.store.sign_in();
        match h.last_state() {
            AuthState::SignedOut {
                reason: SignedOutReason::Error,
                detail: Some(detail),
            } => assert!(detail.contains("Invalid authorization code")),
            other => panic!("unexpected {other:?}"),
        }
    }

    // report_not_premium

    #[test]
    fn publishes_not_premium_and_keeps_the_refresh_token() {
        let h = build(Build::default());
        h.store.start().unwrap();
        h.store.report_not_premium();
        assert_eq!(
            h.store.state(),
            AuthState::signed_out(SignedOutReason::NotPremium)
        );
        assert_eq!(h.keychain.deletes(), 0);
        assert_eq!(h.keychain.stored.lock().unwrap().as_deref(), Some("rt-0"));
    }

    // on_state

    #[test]
    fn stops_delivering_after_the_subscription_is_dropped() {
        let h = build(Build {
            stored: None,
            ..Default::default()
        });
        let seen = Arc::new(Mutex::new(Vec::new()));
        let sub = h.store.on_state({
            let seen = seen.clone();
            move |s| seen.lock().unwrap().push(s.clone())
        });
        drop(sub);
        h.store.start().unwrap();
        assert!(seen.lock().unwrap().is_empty());
    }
}
