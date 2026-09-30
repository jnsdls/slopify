// The shapes docs/spec/v1.md fixes for the bridge. The serde attributes keep the JSON identical to
// the TypeScript types, because the state file and the Player's web view both carry them as JSON.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(
    tag = "kind",
    rename_all = "lowercase",
    rename_all_fields = "camelCase"
)]
pub enum Source {
    Playlist {
        id: String,
        uri: String,
        name: String,
        image_url: Option<String>,
        pasted: bool,
    },
    Liked,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ResumePoint {
    pub source: Source,
    pub track_uri: Option<String>,
    pub position_ms: u64,
}

/// Errors that reach the UI carry a `code`; the UI switches on it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum BridgeErrorCode {
    BadLink,
    NotFound,
    Forbidden,
    NoDevice,
    PlayFailed,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BridgeError {
    pub code: BridgeErrorCode,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub status: Option<u16>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
}

impl BridgeError {
    pub fn new(code: BridgeErrorCode) -> Self {
        Self {
            code,
            status: None,
            message: None,
        }
    }

    pub fn with_status(code: BridgeErrorCode, status: u16) -> Self {
        Self {
            code,
            status: Some(status),
            message: None,
        }
    }
}

impl std::fmt::Display for BridgeError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // The JSON form, so a logged error reads the same as it did in the Electron app.
        f.write_str(&serde_json::to_string(self).map_err(|_| std::fmt::Error)?)
    }
}

impl std::error::Error for BridgeError {}

/// What another Connect device is playing, from GET /me/player. Feeds the "Playing on" state.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PlayingElsewhere {
    pub device_name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub track: Option<ElsewhereTrack>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ElsewhereTrack {
    pub uri: String,
    pub name: String,
    pub artists: Vec<Artist>,
    pub album: String,
    pub image_url: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Artist {
    pub name: String,
    pub uri: String,
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn source_json_matches_the_typescript_shape() {
        let playlist = Source::Playlist {
            id: "abc".into(),
            uri: "spotify:playlist:abc".into(),
            name: "Mix".into(),
            image_url: None,
            pasted: false,
        };
        assert_eq!(
            serde_json::to_string(&playlist).unwrap(),
            r#"{"kind":"playlist","id":"abc","uri":"spotify:playlist:abc","name":"Mix","imageUrl":null,"pasted":false}"#
        );
        assert_eq!(
            serde_json::to_string(&Source::Liked).unwrap(),
            r#"{"kind":"liked"}"#
        );
    }

    #[test]
    fn bridge_error_omits_absent_fields() {
        let e = BridgeError::with_status(BridgeErrorCode::PlayFailed, 404);
        assert_eq!(
            serde_json::to_value(&e).unwrap(),
            json!({ "code": "play-failed", "status": 404 })
        );
        let e = BridgeError::new(BridgeErrorCode::BadLink);
        assert_eq!(
            serde_json::to_value(&e).unwrap(),
            json!({ "code": "bad-link" })
        );
    }

    #[test]
    fn bridge_error_rejects_unknown_codes() {
        assert!(serde_json::from_value::<BridgeError>(json!({ "code": "nope" })).is_err());
        let parsed: BridgeError = serde_json::from_value(json!({ "code": "not-found" })).unwrap();
        assert_eq!(parsed, BridgeError::new(BridgeErrorCode::NotFound));
    }
}
