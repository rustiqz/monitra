//! Colors and shared styles (DESIGN.md §4 `tui`). Carried over unchanged
//! from the `feat/tui-design` visual reference (Phase 9).

use ratatui::style::{Color, Modifier, Style};

pub const BG: Color = Color::Rgb(8, 11, 16);
pub const BAR: Color = Color::Rgb(11, 16, 23);
pub const BORDER: Color = Color::Rgb(29, 43, 58);
pub const MUTED: Color = Color::Rgb(109, 131, 149);
pub const TEXT: Color = Color::Rgb(184, 199, 212);
pub const BRIGHT: Color = Color::Rgb(230, 240, 247);
pub const CYAN: Color = Color::Rgb(95, 211, 227);
pub const GREEN: Color = Color::Rgb(95, 227, 161);
pub const RED: Color = Color::Rgb(240, 112, 138);
pub const AMBER: Color = Color::Rgb(242, 193, 78);
pub const VIOLET: Color = Color::Rgb(138, 111, 214);
pub const PENDING: Color = Color::Rgb(176, 140, 245);

pub fn title() -> Style {
    Style::default().fg(CYAN).add_modifier(Modifier::BOLD)
}

pub fn dim() -> Style {
    Style::default().fg(MUTED)
}

pub fn selected() -> Style {
    Style::default()
        .fg(BG)
        .bg(CYAN)
        .add_modifier(Modifier::BOLD)
}

/// Maps a `MonitorStatus` to its display color — the one place status→color
/// is decided, so `ui.rs` never hardcodes it per screen.
pub fn status_color(status: monitra_models::MonitorStatus) -> Color {
    use monitra_models::MonitorStatus::*;
    match status {
        Up => GREEN,
        Down => RED,
        Pending => PENDING,
        Paused => MUTED,
        Stale => VIOLET,
    }
}
