//! The Electron renderer's CSS custom properties, one set per system appearance.

use gpui::{Hsla, WindowAppearance, rgb, rgba};

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Theme {
    pub bg: Hsla,
    pub fg: Hsla,
    pub muted: Hsla,
    pub line: Hsla,
    pub hover: Hsla,
    pub track: Hsla,
    pub field: Hsla,
    pub field_line: Hsla,
    pub tile: Hsla,
    pub play_bg: Hsla,
    pub play_fg: Hsla,
    pub error: Hsla,
    pub selection: Hsla,
}

impl Theme {
    pub fn for_appearance(appearance: WindowAppearance) -> Self {
        match appearance {
            WindowAppearance::Dark | WindowAppearance::VibrantDark => Self::dark(),
            WindowAppearance::Light | WindowAppearance::VibrantLight => Self::light(),
        }
    }

    fn dark() -> Self {
        Self {
            bg: rgb(0x1e1e21).into(),
            fg: rgb(0xf2f2f2).into(),
            muted: rgb(0x9a9a9e).into(),
            line: rgba(0xffffff1a).into(),
            hover: rgba(0xffffff12).into(),
            track: rgba(0xffffff2e).into(),
            field: rgba(0xffffff14).into(),
            field_line: rgba(0xffffff24).into(),
            tile: rgb(0x3a3a3f).into(),
            play_bg: rgb(0xffffff).into(),
            play_fg: rgb(0x000000).into(),
            error: rgb(0xff6b6b).into(),
            selection: rgba(0x3f638b99).into(),
        }
    }

    fn light() -> Self {
        Self {
            bg: rgb(0xf4f4f6).into(),
            fg: rgb(0x1a1a1c).into(),
            muted: rgb(0x6b6b70).into(),
            line: rgba(0x0000001a).into(),
            hover: rgba(0x0000000d).into(),
            track: rgba(0x00000029).into(),
            field: rgba(0x0000000d).into(),
            field_line: rgba(0x00000024).into(),
            tile: rgb(0xd8d8dc).into(),
            play_bg: rgb(0x1a1a1c).into(),
            play_fg: rgb(0xffffff).into(),
            error: rgb(0xc92a2a).into(),
            selection: rgba(0xb3d7ffcc).into(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn follows_the_system_appearance() {
        assert_eq!(Theme::for_appearance(WindowAppearance::Dark), Theme::dark());
        assert_eq!(
            Theme::for_appearance(WindowAppearance::VibrantDark),
            Theme::dark()
        );
        assert_eq!(
            Theme::for_appearance(WindowAppearance::Light),
            Theme::light()
        );
        assert_eq!(
            Theme::for_appearance(WindowAppearance::VibrantLight),
            Theme::light()
        );
    }
}
