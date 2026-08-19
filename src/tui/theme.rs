use ratatui::style::Color;
use std::sync::OnceLock;

use crate::config;

/// Not user-configurable: used only for the week-view day-separator rows.
pub const DAY_HEADER_BG: Color = Color::Rgb(38, 48, 68); // Dark blue for day separators

/// Not user-configurable: used only for the background of modal popups.
pub const OVERLAY_BG: Color = Color::Rgb(28, 28, 28);

pub struct Theme {
    pub accent: Color,
    pub active: Color,
    pub inactive: Color,
    pub header_bg: Color,
    pub selected_bg: Color,
    pub highlight: Color,
    pub duration_high: Color,
    pub duration_med: Color,
    pub duration_low: Color,
    pub border: Color,
    pub title: Color,

    /// Thresholds (in hours) for coloring a single time entry's duration.
    pub entry_duration_high_h: i64,
    pub entry_duration_med_h: i64,

    /// Thresholds (in hours) for coloring a day's total tracked duration.
    pub day_duration_high_h: i64,
    pub day_duration_med_h: i64,
}

impl Theme {
    fn from_config(cfg: &config::ThemeConfig, dur: &config::DurationConfig) -> Self {
        let color = |s: &Option<String>, default: (u8, u8, u8)| {
            s.as_deref()
                .and_then(config::parse_hex_rgb)
                .map(|(r, g, b)| Color::Rgb(r, g, b))
                .unwrap_or(Color::Rgb(default.0, default.1, default.2))
        };

        Theme {
            accent: color(&cfg.accent, (138, 180, 248)), // Light blue
            active: color(&cfg.active, (129, 199, 132)), // Green
            inactive: color(&cfg.inactive, (144, 144, 144)), // Gray
            header_bg: color(&cfg.header_bg, (48, 48, 48)), // Dark gray
            selected_bg: color(&cfg.selected_bg, (66, 66, 66)), // Medium gray
            highlight: color(&cfg.highlight, (255, 213, 79)), // Yellow/gold
            duration_high: color(&cfg.duration_high, (239, 154, 154)), // Light red
            duration_med: color(&cfg.duration_med, (255, 224, 130)), // Light yellow
            duration_low: color(&cfg.duration_low, (165, 214, 167)), // Light green
            border: color(&cfg.border, (88, 88, 88)), // Border gray
            title: color(&cfg.title, (186, 186, 186)), // Light gray

            entry_duration_high_h: dur.entry_high_hours.unwrap_or(4),
            entry_duration_med_h: dur.entry_med_hours.unwrap_or(2),
            day_duration_high_h: dur.day_high_hours.unwrap_or(8),
            day_duration_med_h: dur.day_med_hours.unwrap_or(4),
        }
    }
}

static THEME: OnceLock<Theme> = OnceLock::new();

pub fn theme() -> &'static Theme {
    THEME.get_or_init(|| {
        let cfg = config::load();
        Theme::from_config(&cfg.theme, &cfg.duration)
    })
}

pub fn accent() -> Color {
    theme().accent
}
pub fn active() -> Color {
    theme().active
}
pub fn inactive() -> Color {
    theme().inactive
}
pub fn header_bg() -> Color {
    theme().header_bg
}
pub fn selected_bg() -> Color {
    theme().selected_bg
}
pub fn highlight() -> Color {
    theme().highlight
}
pub fn border() -> Color {
    theme().border
}
pub fn title() -> Color {
    theme().title
}

/// Maps a duration (in hours) to a color given high/medium thresholds.
pub fn duration_color(hours: i64, high_threshold: i64, med_threshold: i64) -> Color {
    let t = theme();
    if hours >= high_threshold {
        t.duration_high
    } else if hours >= med_threshold {
        t.duration_med
    } else {
        t.duration_low
    }
}
