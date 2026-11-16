use super::overlay::{CURSOR_MARKER, overlay_hints, render_overlay};
use crate::marks::Mark;
use crate::tui::{App, theme};
use ratatui::{prelude::*, widgets::Paragraph};

/// The `x` popup: one row per open mark, never capped at the surface's three.
pub(super) fn render_mark_picker_popup(f: &mut Frame, app: &App) {
    let lines: Vec<Line> = app
        .marks
        .iter()
        .enumerate()
        .map(|(index, mark)| {
            let selected = index == app.mark_cursor;
            let (marker, style) = if selected {
                (
                    CURSOR_MARKER,
                    Style::default().fg(theme::highlight()).bold(),
                )
            } else {
                ("   ", Style::default().fg(theme::active()))
            };
            Line::from(vec![
                Span::styled(marker, Style::default().fg(theme::accent()).bold()),
                Span::styled(mark_summary(mark), style),
            ])
        })
        .collect();
    render_mark_popup(
        f,
        " Open marks ",
        lines,
        overlay_hints(&[("j/k", "move"), ("enter", "choose"), ("esc", "cancel")]),
    );
}

/// The prompt for the picked mark. It draws the captured mark, not the cursor row.
pub(super) fn render_mark_action_popup(f: &mut Frame, app: &App) {
    let Some(mark) = app.picked_mark.as_ref() else {
        return;
    };
    render_mark_popup(
        f,
        " Close mark ",
        vec![Line::from(Span::styled(
            format!("   {}", mark_summary(mark)),
            Style::default().fg(theme::highlight()).bold(),
        ))],
        overlay_hints(&[("e", "edit and log"), ("d", "discard"), ("esc", "cancel")]),
    );
}

fn mark_summary(mark: &Mark) -> String {
    format!("{}  {}", mark.label(), mark.elapsed())
}

fn render_mark_popup(f: &mut Frame, title: &'static str, lines: Vec<Line>, hints: Line<'static>) {
    let widest = lines
        .iter()
        .map(Line::width)
        .max()
        .unwrap_or(0)
        .max(hints.width());
    let width = (widest as u16 + 4).clamp(40, 100);
    let content = render_overlay(
        f,
        width,
        lines.len() as u16 + 3,
        Span::styled(title, Style::default().fg(theme::highlight()).bold()),
        Span::styled(" x ", Style::default().fg(theme::inactive())),
        hints,
    );
    f.render_widget(
        Paragraph::new(lines).style(Style::default().bg(theme::overlay_bg())),
        content,
    );
}
