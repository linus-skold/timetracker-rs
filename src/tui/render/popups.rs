use super::overlay::{overlay_height, overlay_hints, render_overlay, wrap};
use crate::tui::types::ConfirmAction;
use crate::tui::{App, theme};
use chrono::Duration;
use ratatui::{prelude::*, widgets::Paragraph};

/// Section title, then `(keys, description)` rows. The key column is one
/// width across every section, measured in display cells.
const HELP_SECTIONS: &[(&str, &[(&str, &str)])] = &[
    (
        "Navigation",
        &[
            ("h / ←", "previous period"),
            ("l / →", "next period"),
            ("j / ↓", "select next entry"),
            ("k / ↑", "select previous entry"),
            ("t", "go to today"),
            ("1 / 2 / 3 / 4", "day / week / all / overview"),
        ],
    ),
    (
        "Entries",
        &[
            ("a", "add entry (uses browsed date)"),
            ("e", "edit selected entry"),
            ("d", "delete selected entry (asks first)"),
            ("s", "stop active entry"),
            ("Enter", "entry detail"),
            // A path, not a key: the trim is live only while the popover is open.
            ("Enter, t", "trim idle from the entry (asks first)"),
        ],
    ),
    (
        "Search & Filter",
        &[
            ("/", "search any field"),
            ("Shift-P", "Projects pane on / off"),
            ("Shift-T", "Tags pane on / off"),
            ("Tab", "focus table / panes"),
            ("Shift-Tab", "focus panes in reverse"),
            ("Enter", "pane value: include / exclude / off"),
            ("-", "cycle the pane value back"),
        ],
    ),
    (
        "Other",
        &[
            ("Shift-A", "agent phases on / off"),
            ("Shift-S", "project summary on / off"),
            ("o", "toggle sort order"),
            ("r", "reload data from disk"),
            ("?", "toggle this help"),
            ("q / Esc", "quit"),
        ],
    ),
];

/// Width and height follow the table, and `app.help_scroll` is clamped here
/// against the viewport `render_overlay` actually granted — never in the key
/// handler — so a resize corrects itself on the next frame.
pub(super) fn render_help_popup(f: &mut Frame, app: &mut App) {
    const INSET: &str = "  ";
    const GAP: usize = 2;
    let key_style = Style::default().fg(theme::accent()).bold();
    let desc_style = Style::default().fg(theme::inactive());
    let heading_style = Style::default().fg(theme::highlight()).bold();

    let rows = HELP_SECTIONS.iter().flat_map(|(_, rows)| rows.iter());
    let key_width = rows
        .clone()
        .map(|(k, _)| Span::raw(*k).width())
        .max()
        .unwrap_or(0);
    let desc_width = rows.map(|(_, d)| Span::raw(*d).width()).max().unwrap_or(0);

    // A blank row above and below the table keeps it off the title and hints.
    let mut lines: Vec<Line> = vec![Line::from("")];
    for (i, (title, rows)) in HELP_SECTIONS.iter().enumerate() {
        if i > 0 {
            lines.push(Line::from(""));
        }
        lines.push(Line::from(Span::styled(
            format!("{INSET}{title}"),
            heading_style,
        )));
        for (k, d) in rows.iter() {
            let pad = key_width - Span::raw(*k).width() + GAP;
            lines.push(Line::from(vec![
                Span::raw(INSET),
                Span::styled(*k, key_style),
                Span::raw(" ".repeat(pad)),
                Span::styled(*d, desc_style),
            ]));
        }
    }
    lines.push(Line::from(""));

    let width = (INSET.len() + key_width + GAP + desc_width + 2).max(32) as u16;
    let wanted = lines.len() as u16 + 3;
    let fits = overlay_height(f, wanted) == wanted;
    let hints: &[(&str, &str)] = if fits {
        &[("esc", "close")]
    } else {
        &[("esc", "close"), ("j/k", "scroll")]
    };
    let content = render_overlay(
        f,
        width,
        wanted,
        Span::styled(" Keybindings ", heading_style),
        Span::styled(" ? ", desc_style),
        overlay_hints(hints),
    );

    let visible = content.height as usize;
    let max_offset = lines.len().saturating_sub(visible);
    app.help_scroll = app.help_scroll.min(max_offset);
    let more_below = app.help_scroll < max_offset;
    let shown = if more_below {
        visible.saturating_sub(1)
    } else {
        visible
    };
    let mut page: Vec<Line> = lines
        .into_iter()
        .skip(app.help_scroll)
        .take(shown)
        .collect();
    if more_below {
        page.push(Line::from(Span::styled("▾ more", desc_style)).right_aligned());
    }
    f.render_widget(
        Paragraph::new(page).style(Style::default().bg(theme::OVERLAY_BG)),
        content,
    );
}

/// A destructive action waiting for a yes. **It names the entry**, from the
/// *captured* id and never `selected_entry()`, which the poll can move. The trim
/// outcome comes from `TimeEntry::trim_spans` — never a second copy of that here.
pub(super) fn render_confirm_popup(f: &mut Frame, app: &App) {
    let Some(pending) = app.pending_confirm else {
        return;
    };
    // Gone from under the prompt: draw nothing rather than the wrong entry.
    let Some(entry) = app.data.get_entry(pending.entry_id) else {
        return;
    };

    let value = Style::default().fg(Color::White);
    let dim = Style::default().fg(theme::inactive());

    // The subject line, so a cursor on the wrong row shows up here.
    let mut subject = vec![Span::styled(format!("  {}", entry.description), value)];
    if let Some(project) = entry.project.as_deref().filter(|p| !p.trim().is_empty()) {
        subject.push(Span::styled(format!(" ({})", project), dim));
    }
    subject.push(Span::styled(
        format!("  {}", entry.format_duration()),
        Style::default().fg(theme::accent()).bold(),
    ));
    let mut lines = vec![Line::from(subject)];

    if pending.action == ConfirmAction::Trim {
        let pieces = entry.trim_spans();
        let kept = pieces.iter().fold(Duration::zero(), |acc, (from, to)| {
            acc + to.signed_duration_since(*from)
        });
        let durations: Vec<String> = pieces
            .iter()
            .map(|(from, to)| crate::duration::format(to.signed_duration_since(*from)))
            .collect();
        lines.push(Line::from(vec![
            Span::styled("  -> ", dim),
            Span::styled(format!("{} pieces: ", durations.len()), value),
            Span::styled(durations.join(", "), Style::default().fg(theme::accent())),
        ]));
        lines.push(Line::from(vec![
            Span::styled("     ", dim),
            Span::styled(
                crate::duration::format(entry.duration() - kept),
                Style::default().fg(theme::accent()).bold(),
            ),
            Span::styled(" removed", dim),
        ]));
    }

    // Sized from the content but bounded, so it never becomes a full-width banner.
    let title = format!(" {} entry #{}? ", pending.action.verb(), entry.id);
    let widest = lines
        .iter()
        .map(Line::width)
        .chain(std::iter::once(title.chars().count()))
        .max()
        .unwrap_or(0);
    let width = (widest as u16 + 4).clamp(32, 72);

    let content = render_overlay(
        f,
        width,
        lines.len() as u16 + 3,
        Span::styled(title, Style::default().fg(theme::highlight()).bold()),
        Span::styled(" confirm ", Style::default().fg(theme::inactive())),
        // Exactly what the `Confirm` arm binds, `enter` among the cancels.
        overlay_hints(&[
            (pending.action.confirm_keys(), "yes"),
            ("n / esc / enter", "cancel"),
        ]),
    );
    f.render_widget(
        Paragraph::new(lines).style(Style::default().bg(theme::OVERLAY_BG)),
        content,
    );
}

/// The selected entry, every field wrapped rather than truncated.
pub(super) fn render_detail_popup(f: &mut Frame, app: &App) {
    let Some(entry) = app.selected_entry() else {
        return;
    };

    // Wide enough for the longest label, so values line up.
    const LABEL: usize = 12;
    let width = 80u16.min(f.area().width.saturating_sub(4)).max(24) as usize;
    // Hand-counted: 2 borders + 2 indent + label + 2 right margin.
    let value_width = width.saturating_sub(2 + 2 + LABEL + 2).max(8);

    fn label(text: &str) -> Span<'_> {
        Span::styled(
            format!("  {:<width$}", text, width = LABEL),
            Style::default().fg(theme::title()),
        )
    }

    /// One labelled field, continuation lines indented to the value column.
    fn field(name: &str, value: &str, style: Style, value_width: usize) -> Vec<Line<'static>> {
        wrap(value, value_width)
            .into_iter()
            .enumerate()
            .map(|(i, text)| {
                let head = if i == 0 {
                    format!("  {:<width$}", name, width = LABEL)
                } else {
                    " ".repeat(2 + LABEL)
                };
                Line::from(vec![
                    Span::styled(head, Style::default().fg(theme::title())),
                    Span::styled(text, style),
                ])
            })
            .collect()
    }

    let value = Style::default().fg(Color::White);
    let tags = if entry.tags.is_empty() {
        "—".to_string()
    } else {
        entry.format_tags()
    };
    let mut lines: Vec<Line> = vec![Line::from(Span::raw(""))];
    lines.extend(field(
        "Project",
        entry.project.as_deref().unwrap_or("—"),
        Style::default().fg(theme::accent()),
        value_width,
    ));
    lines.extend(field(
        "Tags",
        &tags,
        Style::default().fg(theme::highlight()),
        value_width,
    ));
    lines.extend(field("Date", &entry.format_date(), value, value_width));
    lines.extend(field(
        "Start",
        &entry.format_start_time(),
        value,
        value_width,
    ));
    // `format_end_time` is already the em dash while an entry runs.
    lines.extend(field("End", &entry.format_end_time(), value, value_width));
    lines.extend(field(
        "Duration",
        &entry.format_duration(),
        Style::default().fg(theme::accent()).bold(),
        value_width,
    ));
    // Idle rows only when the entry has any.
    for (i, gap) in entry.idle.iter().enumerate() {
        lines.push(Line::from(vec![
            label(if i == 0 { "Idle" } else { "" }),
            Span::styled(gap.format_span(), Style::default().fg(theme::inactive())),
        ]));
    }
    lines.push(Line::from(Span::raw("")));
    lines.push(Line::from(label("Description")));
    // The description gets the full inner width, not the value column.
    for text in wrap(&entry.description, width.saturating_sub(2 + 2 + 2).max(8)) {
        lines.push(Line::from(vec![Span::raw("  "), Span::styled(text, value)]));
    }
    lines.push(Line::from(Span::raw("")));

    // The custom JSON, flattened to one labelled row per leaf. Absent entirely
    // when there is no data, so the popover is unchanged for entries without it.
    let data_rows = entry
        .data
        .as_ref()
        .map(crate::entry_data::rows)
        .unwrap_or_default();
    if !data_rows.is_empty() {
        lines.push(Line::from(label("Data")));
        for (key, item) in &data_rows {
            lines.extend(field(
                key,
                item,
                Style::default().fg(theme::highlight()),
                value_width,
            ));
        }
        lines.push(Line::from(Span::raw("")));
    }

    let (marker, marker_style) = if entry.is_active() {
        (" active ", Style::default().fg(theme::active()).bold())
    } else {
        (" logged ", Style::default().fg(theme::inactive()))
    };
    let content = render_overlay(
        f,
        width as u16,
        lines.len() as u16 + 3,
        Span::styled(
            format!(" Entry #{} ", entry.id),
            Style::default().fg(theme::highlight()).bold(),
        ),
        Span::styled(marker, marker_style),
        // Every hint here is bound in the `InputMode::Detail` arm.
        overlay_hints(&app.detail_hints()),
    );
    f.render_widget(
        Paragraph::new(lines).style(Style::default().bg(theme::OVERLAY_BG)),
        content,
    );
}
