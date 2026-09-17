//! The key legend a surface draws on its bottom border.

use crate::tui::{App, theme};
use ratatui::prelude::*;

/// The keys the content box owns, in the way it is drawn: the list sorts and
/// groups, the grid scrolls once it overflows. The toggle leads and is named
/// by the mode it switches **to**, so it reads as the action.
pub(super) fn content_legend(app: &App, width: u16) -> Option<Line<'static>> {
    if app.heat_view {
        let mut keys = vec![("M", "list", true)];
        if app.heat_scroll_max > 0 {
            keys.push(("j/k", "scroll", false));
        }
        return legend(&keys, width);
    }
    legend(
        &[
            ("M", "heatmap", false),
            ("o", "sort order", false),
            ("g", "issue group", false),
        ],
        width,
    )
}

/// Between two entries.
const SEPARATOR: &str = " \u{b7} ";

/// The keys a surface owns, as `(key, label, accented)`, most useful first.
/// The tail sheds while the line is wider than `width`; never a clipped line.
pub(super) fn legend(entries: &[(&str, &str, bool)], width: u16) -> Option<Line<'static>> {
    (1..=entries.len())
        .rev()
        .map(|kept| line(&entries[..kept]))
        .find(|line| line.width() <= width as usize)
}

fn line(entries: &[(&str, &str, bool)]) -> Line<'static> {
    let dim = Style::default().fg(theme::inactive());
    let mut spans = vec![Span::raw(" ")];
    for (index, (key, label, accented)) in entries.iter().enumerate() {
        if index > 0 {
            spans.push(Span::styled(
                SEPARATOR,
                Style::default().fg(theme::border()),
            ));
        }
        let key_style = if *accented {
            Style::default().fg(theme::accent())
        } else {
            dim
        };
        spans.push(Span::styled((*key).to_string(), key_style));
        spans.push(Span::styled(format!(": {label}"), dim));
    }
    spans.push(Span::raw(" "));
    Line::from(spans)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn text(line: &Line<'_>) -> String {
        line.spans
            .iter()
            .map(|span| span.content.as_ref())
            .collect()
    }

    fn pane_entries() -> [(&'static str, &'static str, bool); 3] {
        [
            ("Enter", "filter", false),
            ("-", "back", false),
            ("j/k", "move", false),
        ]
    }

    #[test]
    fn the_whole_legend_is_drawn_while_it_fits() {
        let line = legend(&pane_entries(), 37).expect("the full legend fits 37 cells");
        assert_eq!(
            text(&line),
            " Enter: filter \u{b7} -: back \u{b7} j/k: move "
        );
    }

    #[test]
    fn a_narrow_box_sheds_the_trailing_entries_one_at_a_time() {
        let line = legend(&pane_entries(), 36).expect("two entries fit 36 cells");
        assert_eq!(text(&line), " Enter: filter \u{b7} -: back ");

        let line = legend(&pane_entries(), 24).expect("one entry fits 24 cells");
        assert_eq!(text(&line), " Enter: filter ");
    }

    #[test]
    fn a_box_too_narrow_for_the_first_entry_draws_nothing() {
        assert!(legend(&pane_entries(), 14).is_none());
    }

    /// The toggle names where it goes, not where it is.
    #[test]
    fn the_content_legend_names_the_mode_the_toggle_switches_to() {
        let _guard = crate::storage::env_guard();
        crate::storage::env_sandbox("content-legend");
        crate::storage::save_data(&crate::tracker::TimeData {
            entries: Vec::new(),
            next_id: 0,
            schema_version: 1,
        })
        .unwrap();

        let mut app = App::new().unwrap();
        assert!(!app.heat_view);
        let line = content_legend(&app, 60).expect("the legend fits 60 cells");
        assert!(
            text(&line).starts_with(" M: heatmap "),
            "a list must offer the heatmap: {}",
            text(&line)
        );

        app.heat_view = true;
        let line = content_legend(&app, 60).expect("the legend fits 60 cells");
        assert!(
            text(&line).starts_with(" M: list "),
            "a heatmap must offer the list: {}",
            text(&line)
        );
    }

    /// The grid offers the keys it really has: no sort order, and `j/k` only
    /// once it has more rows than the box.
    #[test]
    fn the_heat_legend_names_the_scroll_only_while_it_scrolls() {
        let _guard = crate::storage::env_guard();
        crate::storage::env_sandbox("content-legend-heat");
        crate::storage::save_data(&crate::tracker::TimeData {
            entries: Vec::new(),
            next_id: 0,
            schema_version: 1,
        })
        .unwrap();

        let mut app = App::new().unwrap();
        app.heat_view = true;
        let line = content_legend(&app, 60).expect("the legend fits 60 cells");
        assert_eq!(text(&line), " M: list ", "a grid that fits offers no keys");

        app.heat_scroll_max = 2;
        let line = content_legend(&app, 60).expect("the legend fits 60 cells");
        assert_eq!(text(&line), " M: list \u{b7} j/k: scroll ");
    }
}
