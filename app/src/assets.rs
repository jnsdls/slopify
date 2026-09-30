//! The SVGs the Dropdown draws, served to GPUI's `svg()` from the binary.

use std::borrow::Cow;

use gpui::{AssetSource, SharedString};

/// The glyphs from the Electron renderer's sprite, all on a 24 x 24 grid.
macro_rules! glyph {
    ($d:literal) => {
        concat!(
            r#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24"><path d=""#,
            $d,
            r#""/></svg>"#
        )
        .as_bytes()
    };
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Icon {
    Play,
    Pause,
    Next,
    Volume,
    Heart,
    Chevron,
    Check,
    AppMark,
    SpotifyLogo,
}

impl Icon {
    const ALL: [Icon; 9] = [
        Icon::Play,
        Icon::Pause,
        Icon::Next,
        Icon::Volume,
        Icon::Heart,
        Icon::Chevron,
        Icon::Check,
        Icon::AppMark,
        Icon::SpotifyLogo,
    ];

    pub fn path(self) -> &'static str {
        match self {
            Icon::Play => "icons/play.svg",
            Icon::Pause => "icons/pause.svg",
            Icon::Next => "icons/next.svg",
            Icon::Volume => "icons/volume.svg",
            Icon::Heart => "icons/heart.svg",
            Icon::Chevron => "icons/chevron.svg",
            Icon::Check => "icons/check.svg",
            Icon::AppMark => "icons/app-icon.svg",
            Icon::SpotifyLogo => "icons/spotify-logo.svg",
        }
    }

    fn bytes(self) -> &'static [u8] {
        match self {
            Icon::Play => glyph!("M8 5v14l11-7z"),
            Icon::Pause => glyph!("M6 5h4v14H6zm8 0h4v14h-4z"),
            Icon::Next => glyph!("M6 18l8.5-6L6 6zM16 6h2v12h-2z"),
            Icon::Volume => {
                glyph!("M3 9v6h4l5 5V4L7 9zm13.5 3A4.5 4.5 0 0 0 14 8v8a4.5 4.5 0 0 0 2.5-4z")
            }
            Icon::Heart => glyph!(
                "M12 21.35l-1.45-1.32C5.4 15.36 2 12.28 2 8.5 2 5.42 4.42 3 7.5 3c1.74 0 3.41.81 4.5 2.09C13.09 3.81 14.76 3 16.5 3 19.58 3 22 5.42 22 8.5c0 3.78-3.4 6.86-8.55 11.54L12 21.35z"
            ),
            Icon::Chevron => glyph!("M7.4 8.6L12 13.2l4.6-4.6L18 10l-6 6-6-6z"),
            Icon::Check => glyph!("M9 16.2l-3.5-3.5L4 14.2l5 5 12-12-1.4-1.4z"),
            Icon::AppMark => include_bytes!("../assets/icons/app-icon.svg"),
            Icon::SpotifyLogo => include_bytes!("../assets/icons/spotify-logo.svg"),
        }
    }
}

pub struct Assets;

impl AssetSource for Assets {
    fn load(&self, path: &str) -> gpui::Result<Option<Cow<'static, [u8]>>> {
        Ok(Icon::ALL
            .iter()
            .find(|icon| icon.path() == path)
            .map(|icon| Cow::Borrowed(icon.bytes())))
    }

    fn list(&self, path: &str) -> gpui::Result<Vec<SharedString>> {
        Ok(Icon::ALL
            .iter()
            .map(|icon| icon.path())
            .filter(|p| p.starts_with(path))
            .map(SharedString::from)
            .collect())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn serves_every_icon_as_an_svg() {
        for icon in Icon::ALL {
            let bytes = Assets.load(icon.path()).unwrap().unwrap();
            assert!(std::str::from_utf8(&bytes).unwrap().contains("<svg"));
        }
        assert!(Assets.load("icons/missing.svg").unwrap().is_none());
    }
}
