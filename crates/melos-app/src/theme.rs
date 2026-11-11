//! Monochrome visual tokens: black, grey and white only.

pub const HOVER: u32 = 0x1D1D1F;
pub const BORDER: u32 = 0x2A2A2C;

pub const TEXT: u32 = 0xF5F5F7;
pub const TEXT_MUTED: u32 = 0x9A9A9E;
pub const TEXT_FAINT: u32 = 0x68686C;

/// Near-black vertical gradient for the window backdrop.
pub fn backdrop() -> gpui::Background {
    gpui::linear_gradient(
        160.,
        gpui::linear_color_stop(gpui::hsla(0., 0., 0.075, 1.0), 0.0),
        gpui::linear_color_stop(gpui::hsla(0., 0., 0.02, 1.0), 1.0),
    )
}

/// Neutral grey cover placeholder, varied by index so the grid still has
/// structure without introducing colour.
pub fn cover(index: usize) -> gpui::Background {
    let base = 0.16 + (index % 4) as f32 * 0.035;
    gpui::linear_gradient(
        135.,
        gpui::linear_color_stop(gpui::hsla(0., 0., base, 1.0), 0.0),
        gpui::linear_color_stop(gpui::hsla(0., 0., (base * 0.55).max(0.05), 1.0), 1.0),
    )
}
