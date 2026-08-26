use crate::tui::theme;
use ratatui::{
    prelude::*,
    widgets::{Block, Borders, Clear, Paragraph},
};

/// The cursor row marker for every list that has one. One constant, so a list's
/// reserved width can never be out of step with what gets drawn.
pub(super) const CURSOR_MARKER: &str = ">> ";

/// Greedy word wrap on spaces; a word longer than `width` is left long.
/// Always returns at least one (possibly empty) line.
pub(super) fn wrap(text: &str, width: usize) -> Vec<String> {
    let mut lines: Vec<String> = Vec::new();
    for word in text.split_whitespace() {
        match lines.last_mut() {
            Some(line) if line.chars().count() + 1 + word.chars().count() <= width => {
                line.push(' ');
                line.push_str(word);
            }
            _ => lines.push(word.to_string()),
        }
    }
    if lines.is_empty() {
        lines.push(String::new());
    }
    lines
}

/// The only place any modal overlay is positioned or framed: a centred, cleared
/// `Rect` with `title` and `marker` on its top border and `hints` on its last row.
/// Returns the content area between the two.
pub(super) fn render_overlay(
    f: &mut Frame,
    width: u16,
    height: u16,
    title: Span<'_>,
    marker: Span<'_>,
    hints: Line<'_>,
) -> Rect {
    let area = f.area();
    let width = width.min(area.width.saturating_sub(4));
    let height = overlay_height(f, height);
    let popup_area = Rect {
        x: (area.width.saturating_sub(width)) / 2,
        y: (area.height.saturating_sub(height)) / 2,
        width,
        height,
    };

    // `Clear` first, or the widgets underneath show through the popup's gaps.
    f.render_widget(Clear, popup_area);
    let overlay_style = Style::default().bg(theme::OVERLAY_BG);
    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(theme::accent()))
        .title(title)
        .title_top(Line::from(marker).right_aligned())
        .style(overlay_style);
    let inner = block.inner(popup_area);
    f.render_widget(block, popup_area);

    let rows = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Min(0), Constraint::Length(1)])
        .split(inner);
    f.render_widget(Paragraph::new(hints).style(overlay_style), rows[1]);
    rows[0]
}

/// The rows an overlay asking for `wanted` actually gets on this frame.
pub(super) fn overlay_height(f: &Frame, wanted: u16) -> u16 {
    wanted.min(f.area().height.saturating_sub(2))
}

/// ` esc close `-style hints, spelled out rather than glyphed.
pub(super) fn overlay_hints(pairs: &[(&'static str, &'static str)]) -> Line<'static> {
    let mut spans = vec![Span::raw(" ")];
    for (i, (k, what)) in pairs.iter().enumerate() {
        if i > 0 {
            spans.push(Span::styled("  ", Style::default().fg(theme::border())));
        }
        spans.push(Span::styled(
            *k,
            Style::default().fg(theme::accent()).bold(),
        ));
        spans.push(Span::styled(
            format!(" {}", what),
            Style::default().fg(theme::inactive()),
        ));
    }
    Line::from(spans)
}
