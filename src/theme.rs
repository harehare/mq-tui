//! Color palettes for the UI. `dark()` matches the original hardcoded colors;
//! `light()` swaps bright ANSI colors for darker ones that stay readable on
//! a white background.

use ratatui::style::Color;

use crate::config::ThemeName;

#[derive(Debug, Clone, Copy)]
pub struct Theme {
    pub accent: Color,
    pub muted: Color,
    pub selection_fg: Color,
    pub selection_bg: Color,
    pub header: Color,
    pub success: Color,
    pub keyword: Color,
    pub selector: Color,
    pub string_lit: Color,
    pub number_lit: Color,
    pub function: Color,
    pub pipe: Color,
    pub comment: Color,
    pub match_fg: Color,
    pub match_bg: Color,
    pub code_fg: Color,
    pub code_bg: Color,
}

impl Theme {
    pub fn from_name(name: ThemeName) -> Self {
        match name {
            ThemeName::Dark => Self::dark(),
            ThemeName::Light => Self::light(),
        }
    }

    pub fn dark() -> Self {
        Self {
            accent: Color::Yellow,
            muted: Color::DarkGray,
            selection_fg: Color::Black,
            selection_bg: Color::White,
            header: Color::Cyan,
            success: Color::Green,
            keyword: Color::Magenta,
            selector: Color::Cyan,
            string_lit: Color::Green,
            number_lit: Color::LightMagenta,
            function: Color::LightBlue,
            pipe: Color::Yellow,
            comment: Color::DarkGray,
            match_fg: Color::Black,
            match_bg: Color::Yellow,
            code_fg: Color::White,
            code_bg: Color::Rgb(40, 40, 40),
        }
    }

    pub fn light() -> Self {
        Self {
            accent: Color::Rgb(150, 100, 0),
            muted: Color::DarkGray,
            selection_fg: Color::White,
            selection_bg: Color::Rgb(0, 90, 200),
            header: Color::Blue,
            success: Color::Rgb(0, 120, 0),
            keyword: Color::Rgb(150, 0, 130),
            selector: Color::Blue,
            string_lit: Color::Rgb(0, 120, 0),
            number_lit: Color::Rgb(130, 0, 150),
            function: Color::Rgb(0, 80, 200),
            pipe: Color::Rgb(150, 100, 0),
            comment: Color::DarkGray,
            match_fg: Color::Black,
            match_bg: Color::Rgb(255, 210, 0),
            code_fg: Color::Black,
            code_bg: Color::Rgb(225, 225, 225),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_from_name_selects_correct_palette() {
        let dark = Theme::from_name(ThemeName::Dark);
        let light = Theme::from_name(ThemeName::Light);
        assert_eq!(dark.accent, Color::Yellow);
        assert_ne!(dark.accent, light.accent);
    }
}
