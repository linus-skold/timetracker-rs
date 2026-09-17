use ratatui::style::Color;
use std::sync::OnceLock;

use crate::config;

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

    /// Group-header rows in the entries table.
    pub group_header_bg: Color,
    /// The faint tint on an expanded group's member rows.
    pub member_bg: Color,
    /// Week-view day-separator rows.
    pub day_header_bg: Color,
    /// Background of modal popups.
    pub overlay_bg: Color,

    /// The "no time tracked" cell in the yearly overview heatmap.
    pub heatmap_empty: Color,
    /// Green ramp, lightest to most saturated, for the yearly overview heatmap.
    pub heatmap_levels: [Color; 4],

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

            group_header_bg: color(&cfg.group_header_bg, (48, 40, 62)), // Purple
            member_bg: color(&cfg.member_bg, (32, 28, 44)),             // Dark purple
            day_header_bg: color(&cfg.day_header_bg, (38, 48, 68)),     // Dark blue
            overlay_bg: color(&cfg.overlay_bg, (28, 28, 28)),           // Near-black

            heatmap_empty: color(&cfg.heatmap_empty, (45, 45, 45)),
            heatmap_levels: [
                color(&cfg.heatmap_level1, (155, 233, 168)),
                color(&cfg.heatmap_level2, (64, 196, 99)),
                color(&cfg.heatmap_level3, (48, 161, 78)),
                color(&cfg.heatmap_level4, (33, 110, 57)),
            ],

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
pub fn group_header_bg() -> Color {
    theme().group_header_bg
}
pub fn member_bg() -> Color {
    theme().member_bg
}
pub fn day_header_bg() -> Color {
    theme().day_header_bg
}
pub fn overlay_bg() -> Color {
    theme().overlay_bg
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

/// Maps a day's tracked hours to one of five progressively more intense
/// greens, for the yearly overview heatmap. Reuses the day-duration
/// thresholds (so a heatmap cell and a weekly-breakdown row agree on what
/// counts as a light/heavy day) but not their colors, which are independent
/// of the app's red/amber/green "duration warning" palette used elsewhere
/// (`duration_color`) — this pane reads as a contribution graph, not a
/// warning about long days.
pub fn heat_color(hours: i64) -> Color {
    let t = theme();
    if hours <= 0 {
        t.heatmap_empty
    } else if hours < t.day_duration_med_h / 2 {
        t.heatmap_levels[0]
    } else if hours < t.day_duration_med_h {
        t.heatmap_levels[1]
    } else if hours < t.day_duration_high_h {
        t.heatmap_levels[2]
    } else {
        t.heatmap_levels[3]
    }
}

/// Maps `part` onto the same ramp by its proportion of `max`, in quarters, for
/// grids whose buckets are too small for [`heat_color`]'s day thresholds. Any
/// unit, as long as both arguments share it. A part or a maximum at or below
/// zero is empty, so an empty grid cannot divide by zero.
pub fn heat_shade(part: i64, max: i64) -> Color {
    let t = theme();
    if part <= 0 || max <= 0 {
        t.heatmap_empty
    } else if part * 4 <= max {
        t.heatmap_levels[0]
    } else if part * 2 <= max {
        t.heatmap_levels[1]
    } else if part * 4 <= max * 3 {
        t.heatmap_levels[2]
    } else {
        t.heatmap_levels[3]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Quarters of the maximum, each band closed at its top.
    #[test]
    fn heat_shade_bands_the_proportion_of_the_maximum() {
        let t = theme();
        assert_eq!(
            heat_shade(25, 100),
            t.heatmap_levels[0],
            "the first quarter"
        );
        assert_eq!(heat_shade(26, 100), t.heatmap_levels[1]);
        assert_eq!(
            heat_shade(50, 100),
            t.heatmap_levels[1],
            "the second quarter"
        );
        assert_eq!(heat_shade(51, 100), t.heatmap_levels[2]);
        assert_eq!(
            heat_shade(75, 100),
            t.heatmap_levels[2],
            "the third quarter"
        );
        assert_eq!(heat_shade(76, 100), t.heatmap_levels[3]);
        assert_eq!(heat_shade(100, 100), t.heatmap_levels[3], "the maximum");
    }

    /// A small part of a small maximum still reads hot: the scale is relative.
    #[test]
    fn heat_shade_rescales_to_whatever_the_maximum_is() {
        let t = theme();
        assert_eq!(heat_shade(1, 1), t.heatmap_levels[3]);
        assert_eq!(heat_shade(1, 4), t.heatmap_levels[0]);
    }

    #[test]
    fn heat_shade_treats_an_empty_bucket_or_an_empty_grid_as_empty() {
        let t = theme();
        assert_eq!(heat_shade(0, 100), t.heatmap_empty, "nothing in the bucket");
        assert_eq!(heat_shade(-5, 100), t.heatmap_empty, "never below empty");
        assert_eq!(heat_shade(10, 0), t.heatmap_empty, "no divide by zero");
        assert_eq!(heat_shade(0, 0), t.heatmap_empty);
    }

    #[test]
    fn heat_color_covers_each_threshold_boundary_with_progressively_greener_shades() {
        let t = theme();
        assert_eq!(heat_color(0), t.heatmap_empty, "no time tracked");
        assert_eq!(
            heat_color(1),
            t.heatmap_levels[0],
            "well under the medium threshold"
        );
        assert_eq!(
            heat_color(t.day_duration_med_h - 1),
            t.heatmap_levels[1],
            "just under the medium threshold"
        );
        assert_eq!(
            heat_color(t.day_duration_med_h),
            t.heatmap_levels[2],
            "at the medium threshold"
        );
        assert_eq!(
            heat_color(t.day_duration_high_h),
            t.heatmap_levels[3],
            "at the high threshold"
        );
    }
}
