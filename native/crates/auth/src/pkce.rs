use base64::Engine;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use rand::RngCore;
use sha2::{Digest, Sha256};

pub const REDIRECT_URI: &str = "http://127.0.0.1:8888/callback";
pub const CALLBACK_PORT: u16 = 8888;
pub const SCOPES: [&str; 7] = [
    "streaming",
    "user-read-email",
    "user-read-private",
    "user-read-playback-state",
    "user-modify-playback-state",
    "playlist-read-private",
    "playlist-read-collaborative",
];

const AUTHORIZE_ENDPOINT: &str = "https://accounts.spotify.com/authorize";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Pkce {
    pub verifier: String,
    pub challenge: String,
    pub state: String,
}

pub fn compute_challenge(verifier: &str) -> String {
    URL_SAFE_NO_PAD.encode(Sha256::digest(verifier.as_bytes()))
}

pub fn generate_pkce() -> Pkce {
    generate_pkce_with(|buf| rand::rng().fill_bytes(buf))
}

/// `fill` stands in for the OS random source so tests can pin the bytes.
pub fn generate_pkce_with(mut fill: impl FnMut(&mut [u8])) -> Pkce {
    let mut verifier = [0u8; 64];
    fill(&mut verifier);
    let verifier = URL_SAFE_NO_PAD.encode(verifier);
    let mut state = [0u8; 16];
    fill(&mut state);
    Pkce {
        challenge: compute_challenge(&verifier),
        verifier,
        state: URL_SAFE_NO_PAD.encode(state),
    }
}

pub fn build_authorize_url(client_id: &str, challenge: &str, state: &str) -> String {
    let scope = SCOPES.join(" ");
    let query = form_urlencoded::Serializer::new(String::new())
        .extend_pairs([
            ("client_id", client_id),
            ("response_type", "code"),
            ("redirect_uri", REDIRECT_URI),
            ("scope", scope.as_str()),
            ("code_challenge_method", "S256"),
            ("code_challenge", challenge),
            ("state", state),
        ])
        .finish();
    format!("{AUTHORIZE_ENDPOINT}?{query}")
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    fn is_base64url(s: &str) -> bool {
        !s.is_empty()
            && s.bytes()
                .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
    }

    #[test]
    fn challenge_matches_the_rfc_7636_appendix_b_vector() {
        assert_eq!(
            compute_challenge("dBjftJeZ4CVP-mB92K27uhbUJU1p1r_wW1gFWFOEjXk"),
            "E9Melhoa2OwvFrEMTJguCHaoeK1t8URWbuGJSstw-cM"
        );
    }

    #[test]
    fn derives_a_64_byte_base64url_verifier_and_its_s256_challenge_from_the_random_source() {
        let mut sizes = Vec::new();
        let p = generate_pkce_with(|buf| {
            sizes.push(buf.len());
            buf.fill(0xfb);
        });
        assert_eq!(sizes[0], 64);
        assert!(is_base64url(&p.verifier));
        assert_eq!(p.verifier, URL_SAFE_NO_PAD.encode([0xfb; 64]));
        assert_eq!(p.challenge, compute_challenge(&p.verifier));
        assert!(is_base64url(&p.state));
    }

    #[test]
    fn gives_a_different_verifier_and_state_per_call() {
        let a = generate_pkce();
        let b = generate_pkce();
        assert_ne!(a.verifier, b.verifier);
        assert_ne!(a.state, b.state);
    }

    fn authorize() -> (String, HashMap<String, String>) {
        let url = build_authorize_url("client-1", "CHAL", "STATE");
        let (base, query) = url.split_once('?').unwrap();
        let params = form_urlencoded::parse(query.as_bytes())
            .into_owned()
            .collect();
        (base.to_string(), params)
    }

    #[test]
    fn targets_the_spotify_authorize_endpoint() {
        assert_eq!(authorize().0, "https://accounts.spotify.com/authorize");
    }

    #[test]
    fn carries_the_client_id_redirect_response_type_and_pkce_fields() {
        let q = authorize().1;
        assert_eq!(q["client_id"], "client-1");
        assert_eq!(q["redirect_uri"], REDIRECT_URI);
        assert_eq!(q["response_type"], "code");
        assert_eq!(q["code_challenge_method"], "S256");
        assert_eq!(q["code_challenge"], "CHAL");
        assert_eq!(q["state"], "STATE");
    }

    #[test]
    fn carries_every_scope() {
        let q = authorize().1;
        let scopes: Vec<&str> = q["scope"].split(' ').collect();
        assert_eq!(scopes, SCOPES);
    }
}
