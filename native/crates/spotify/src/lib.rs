//! The Spotify Web API client: requests with the spec's retry rules, the Source catalog and pasted
//! links, and the playback commands that go through the Web API. Everything blocks; the app calls
//! it from background threads.

mod api;
mod http;
mod playback;
mod sources;
mod types;

#[cfg(test)]
mod fake_api;
#[cfg(test)]
mod test_log;

pub use api::{
    ApiError, ApiResponse, BoxError, Error, HttpClient, HttpRequest, HttpResponse, Method,
    RequestOpts, SpotifyApi, TokenFns, TokenSource, WebApi,
};
pub use http::UreqClient;
pub use playback::{
    Offset, PlayBody, Resume, StartSource, build_play_body, get_playing_elsewhere, start_source,
    transfer_here,
};
pub use sources::{SourceCatalog, parse_pasted_link, resolve_pasted_link};
pub use types::{
    Artist, BridgeError, BridgeErrorCode, ElsewhereTrack, PlayingElsewhere, ResumePoint, Source,
};
