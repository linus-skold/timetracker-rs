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
            border: color(&cfg.border, (88, 88, 88)),    // Border gray
            title: color(&cfg.title, (186, 186, 186)),   // Light gray

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

/// Not user-configurable: the "no time tracked" cell in the yearly overview
/// heatmap.
pub const HEATMAP_EMPTY: Color = Color::Rgb(45, 45, 45);

/// Not user-configurable: a GitHub-style green ramp, lightest to most
/// saturated, for the yearly overview heatmap. Deliberately independent of
/// the app's red/amber/green "duration warning" palette used elsewhere
/// (`duration_color`) — this pane reads as a contribution graph, not a
/// warning about long days.
const HEATMAP_LEVELS: [Color; 4] = [
    Color::Rgb(155, 233, 168),
    Color::Rgb(64, 196, 99),
    Color::Rgb(48, 161, 78),
    Color::Rgb(33, 110, 57),
];

/// Maps a day's tracked hours to one of five progressively more intense
/// greens, for the yearly overview heatmap. Reuses the day-duration
/// thresholds (so a heatmap cell and a weekly-breakdown row agree on what
/// counts as a light/heavy day) but not their colors.
pub fn heat_color(hours: i64) -> Color {
    let t = theme();
    if hours <= 0 {
        HEATMAP_EMPTY
    } else if hours < t.day_duration_med_h / 2 {
        HEATMAP_LEVELS[0]
    } else if hours < t.day_duration_med_h {
        HEATMAP_LEVELS[1]
    } else if hours < t.day_duration_high_h {
        HEATMAP_LEVELS[2]
    } else {
        HEATMAP_LEVELS[3]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn heat_color_covers_each_threshold_boundary_with_progressively_greener_shades() {
        let t = theme();
        assert_eq!(heat_color(0), HEATMAP_EMPTY, "no time tracked");
        assert_eq!(
            heat_color(1),
            HEATMAP_LEVELS[0],
            "well under the medium threshold"
        );
        assert_eq!(
            heat_color(t.day_duration_med_h - 1),
            HEATMAP_LEVELS[1],
            "just under the medium threshold"
        );
        assert_eq!(
            heat_color(t.day_duration_med_h),
            HEATMAP_LEVELS[2],
            "at the medium threshold"
        );
        assert_eq!(
            heat_color(t.day_duration_high_h),
            HEATMAP_LEVELS[3],
            "at the high threshold"
        );
    }
}
