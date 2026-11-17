//! SENZO's native design tokens. Keep visual decisions centralized here.

pub(crate) struct Theme;

impl Theme {
    pub const BACKGROUND: u32 = 0x0A0A0A;
    pub const SIDEBAR: u32 = 0x101112;
    pub const SURFACE: u32 = 0x151718;
    pub const SURFACE_RAISED: u32 = 0x1B1F20;
    pub const SURFACE_HOVER: u32 = 0x202728;
    pub const BORDER: u32 = 0x292F30;
    pub const BORDER_STRONG: u32 = 0x3B4547;
    pub const TEXT_PRIMARY: u32 = 0xF5F7F8;
    pub const TEXT_SECONDARY: u32 = 0xB3BEC1;
    pub const TEXT_MUTED: u32 = 0x7F8B8E;
    pub const ACCENT: u32 = 0x22D3EE;
    pub const ACCENT_DARK: u32 = 0x12343A;
    pub const SUCCESS: u32 = 0x4ADE80;
    pub const WARNING: u32 = 0xFBBF24;
    pub const DANGER: u32 = 0xF87171;
    pub const INFO: u32 = 0x60A5FA;

    pub const SIDEBAR_WIDTH: f32 = 236.0;
    pub const HEADER_HEIGHT: f32 = 84.0;
    pub const CONTENT_PADDING: f32 = 36.0;
    pub const MIN_WINDOW_WIDTH: f32 = 1024.0;
    pub const MIN_WINDOW_HEIGHT: f32 = 680.0;

    pub const RADIUS_SMALL: f32 = 8.0;
    pub const RADIUS_MEDIUM: f32 = 12.0;
    pub const RADIUS_LARGE: f32 = 18.0;
}

#[cfg(test)]
mod tests {
    use super::Theme;

    #[test]
    fn brand_tokens_match_the_senzo_identity() {
        assert_eq!(Theme::BACKGROUND, 0x0A0A0A);
        assert_eq!(Theme::ACCENT, 0x22D3EE);
        assert!(Theme::SIDEBAR_WIDTH >= 200.0);
        assert!(Theme::MIN_WINDOW_WIDTH > Theme::SIDEBAR_WIDTH * 2.0);
    }
}
