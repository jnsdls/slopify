//! Spotify sign-in for slopify: Authorization Code with PKCE through a loopback callback, the
//! refresh token in the login Keychain, and an access token kept fresh in memory.
//!
//! Everything here blocks. Call it from a background thread; nothing needs an async runtime.

mod callback_server;
mod http;
mod keychain;
mod pkce;
mod scheduler;
mod token_store;

pub use callback_server::{CallbackError, CallbackErrorCode, CallbackOptions, CallbackServer};
pub use http::{Http, HttpError, HttpRequest, HttpResponse, Method, UreqHttp};
pub use keychain::{
    KEYCHAIN_SERVICE, Keychain, KeychainError, SecurityKeychain, SecurityOutput, run_security,
};
pub use pkce::{
    CALLBACK_PORT, Pkce, REDIRECT_URI, SCOPES, build_authorize_url, compute_challenge,
    generate_pkce, generate_pkce_with,
};
pub use token_store::{AuthError, AuthState, SignedOutReason, Subscription, TokenStore};
