//! The frame's vertical layout and the top-level draw. Where each surface goes
//! is decided here; how it is drawn lives in the sibling modules.

mod entries;
mod form;
mod onboarding;
mod overlay;
mod popups;
mod surfaces;

use entries::{render_entries_table, render_overview, render_weekly_breakdown};
use form::{render_entry_form, render_search_bar};
use onboarding::render_onboarding_popup;
use popups::{render_confirm_popup, render_detail_popup, render_help_popup};
use surfaces::{render_marks_surface, render_pane_surface, render_summary_surface};

use crate::tracker::TimeData;
use crate::tui::types::{InputMode, ViewMode};
use crate::tui::{App, theme};
use chrono::{Datelike, Duration};
use ratatui::{
    prelude::*,
    widgets::{Block, Borders, Paragraph, Tabs},
};
use std::rc::Rc;

/// A named vertical row of the main layout. Some rows are conditional, so
/// `LayoutRows` resolves a row to its `Rect` by name rather than by index.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum LayoutRow {
    Status,
    Marks,
    Panes,
    Tabs,
    Search,
    Content,
    Summary,
    Footer,
}

/// The rows actually laid out this frame, paired with their areas.
struct LayoutRows {
    names: Vec<LayoutRow>,
    areas: Rc<[Rect]>,
}

impl LayoutRows {
    /// `plan` is the rows top to bottom; a conditional row is left out entirely.
    fn split(area: Rect, plan: Vec<(LayoutRow, Constraint)>) -> Self {
        let (names, constraints): (Vec<LayoutRow>, Vec<Constraint>) = plan.into_iter().unzip();
        let areas = Layout::default()
            .direction(Direction::Vertical)
            .constraints(constraints)
            .split(area);
        Self { names, areas }
    }

    /// The row's area, or `None` when this frame omits it.
    fn get(&self, row: LayoutRow) -> Option<Rect> {
        self.names
            .iter()
            .position(|name| *name == row)
            .map(|idx| self.areas[idx])
    }

    /// For the rows that are always part of the layout.
    fn area(&self, row: LayoutRow) -> Rect {
        self.get(row)
            .unwrap_or_else(|| panic!("layout row {row:?} is always present"))
    }
}

pub fn ui(f: &mut Frame, app: &mut App) {
    let mut plan = vec![(LayoutRow::Status, Constraint::Length(3))];
    let marks_height = app.marks_surface_height();
    if marks_height > 0 {
        plan.push((LayoutRow::Marks, Constraint::Length(marks_height)));
    }
    let pane_height = app.pane_surface_height();
    if pane_height > 0 {
        plan.push((LayoutRow::Panes, Constraint::Length(pane_height)));
    }
    plan.push((LayoutRow::Tabs, Constraint::Length(3))); // Tabs + date info
    if app.is_searching() {
        plan.push((LayoutRow::Search, Constraint::Length(3)));
    }
    plan.push((LayoutRow::Content, Constraint::Min(10)));
    let summary_height = app.summary_surface_height();
    if summary_height > 0 {
        plan.push((LayoutRow::Summary, Constraint::Length(summary_height)));
    }
    plan.push((LayoutRow::Footer, Constraint::Length(3)));
    let rows = LayoutRows::split(f.area(), plan);

    let (status_text, status_style) = match app.data.active_entry() {
        Some(entry) => (
            format!(
                "{}  {} - {} ",
                crate::icons::active(),
                entry.description,
                entry.format_duration()
            ),
            Style::default().fg(theme::active()).bold(),
        ),
        None => (
            "No active task".to_string(),
            Style::default().fg(theme::inactive()).italic(),
        ),
    };
    let mut status_spans = vec![Span::styled(status_text, status_style)];
    if let Some(version) = &app.update_notice {
        status_spans.push(Span::styled(
            format!(
                " | tt {version} available — run `{}`",
                crate::update::update_hint()
            ),
            Style::default().fg(theme::highlight()),
        ));
    }
    let header = Paragraph::new(Line::from(status_spans)).block(
        Block::default()
            .borders(Borders::ALL)
            .border_style(Style::default().fg(theme::border()))
            .title(Span::styled(
                " Status ",
                Style::default().fg(theme::title()),
            )),
    );
    f.render_widget(header, rows.area(LayoutRow::Status));

    if let Some(area) = rows.get(LayoutRow::Marks) {
        render_marks_surface(f, app, area);
    }

    if let Some(area) = rows.get(LayoutRow::Panes) {
        render_pane_surface(f, app, area);
    }

    let tab_titles = vec!["[1] Day", "[2] Week", "[3] All", "[4] Overview"];
    let selected_tab = match app.view_mode {
        ViewMode::Day => 0,
        ViewMode::Week => 1,
        ViewMode::All => 2,
        ViewMode::Overview => 3,
    };
    let date_info = match app.view_mode {
        ViewMode::All => "All entries".to_string(),
        ViewMode::Day => app.selected_date.format("%A, %B %d, %Y").to_string(),
        ViewMode::Week => {
            let week_start = TimeData::week_start(app.selected_date);
            let week_end = week_start + Duration::days(6);
            format!(
                "{} - {}",
                week_start.format("%b %d"),
                week_end.format("%b %d, %Y")
            )
        }
        ViewMode::Overview => format!("Year {}", app.selected_date.year()),
    };
    let tabs = Tabs::new(tab_titles)
        .select(selected_tab)
        .style(Style::default().fg(theme::inactive()))
        .highlight_style(Style::default().fg(theme::accent()).bold())
        .block(
            Block::default()
                .borders(Borders::ALL)
                .border_style(Style::default().fg(theme::border()))
                .title(Span::styled(
                    format!(
                        " {} | {} | {} ",
                        app.view_mode.title(),
                        date_info,
                        app.sort_order.label()
                    ),
                    Style::default().fg(theme::highlight()),
                )),
        );
    f.render_widget(tabs, rows.area(LayoutRow::Tabs));

    if let Some(area) = rows.get(LayoutRow::Search) {
        render_search_bar(f, app, area);
    }

    let content = rows.area(LayoutRow::Content);
    if app.input_mode == InputMode::AddingEntry || app.input_mode == InputMode::EditingEntry {
        render_entry_form(f, app, content);
    } else if app.view_mode == ViewMode::Week {
        let content_chunks = Layout::default()
            .direction(Direction::Horizontal)
            .constraints([Constraint::Length(25), Constraint::Min(40)])
            .split(content);
        render_weekly_breakdown(f, app, content_chunks[0]);
        render_entries_table(f, app, content_chunks[1]);
    } else if app.view_mode == ViewMode::Overview {
        render_overview(f, app, content);
    } else {
        render_entries_table(f, app, content);
    }

    if let Some(area) = rows.get(LayoutRow::Summary) {
        render_summary_surface(f, app, area);
    }

    // Footer: left = hints (clips), right = the key legend (never clips).
    let (total, total_label) = if app.total_is_filtered() {
        (app.filtered_total(), "Filtered: ")
    } else {
        let t = match app.view_mode {
            ViewMode::All => app.data.today_total(),
            ViewMode::Day => app.data.total_for_date(app.selected_date),
            ViewMode::Week => {
                let week_start = TimeData::week_start(app.selected_date);
                app.data.total_for_week(week_start)
            }
            ViewMode::Overview => app
                .data
                .year_breakdown(app.selected_date.year())
                .values()
                .fold(Duration::zero(), |acc, d| acc + *d),
        };
        (t, "Total: ")
    };

    let total_str = crate::duration::format(total);
    let footer = rows.area(LayoutRow::Footer);
    let footer_block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(theme::border()));
    let footer_inner = footer_block.inner(footer);
    f.render_widget(footer_block, footer);

    // Hand-counted against the spans below — update it when they change.
    const KEYS_WIDTH: u16 = 24; // " | P/T/A | Tab | ?: help"
    let hints_width = footer_inner.width.saturating_sub(KEYS_WIDTH);
    let footer_chunks = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Length(hints_width),
            Constraint::Length(KEYS_WIDTH),
        ])
        .split(footer_inner);

    let hint_spans = vec![
        Span::styled(
            format!(" {}", total_label),
            Style::default().fg(theme::title()),
        ),
        Span::styled(total_str, Style::default().fg(theme::highlight()).bold()),
        Span::styled(" | ", Style::default().fg(theme::border())),
        // Detail goes first; this zone clips from the right at 80 columns.
        Span::styled("Enter", Style::default().fg(theme::accent())),
        Span::styled(": detail | ", Style::default().fg(theme::inactive())),
        Span::styled("t", Style::default().fg(theme::accent())),
        Span::styled(": today | ", Style::default().fg(theme::inactive())),
        Span::styled("/", Style::default().fg(theme::accent())),
        Span::styled(": search | ", Style::default().fg(theme::inactive())),
        Span::styled("a", Style::default().fg(theme::accent())),
        Span::styled(": add | ", Style::default().fg(theme::inactive())),
        Span::styled("e", Style::default().fg(theme::accent())),
        Span::styled(": edit | ", Style::default().fg(theme::inactive())),
        Span::styled("d", Style::default().fg(theme::accent())),
        Span::styled(": del… | ", Style::default().fg(theme::inactive())),
        Span::styled("s", Style::default().fg(theme::accent())),
        Span::styled(": stop", Style::default().fg(theme::inactive())),
    ];
    let hints = Paragraph::new(Line::from(hint_spans));
    f.render_widget(hints, footer_chunks[0]);

    // A surface's key is accented while it is open, dim while hidden.
    let key_style = |on: bool| {
        Style::default().fg(if on {
            theme::accent()
        } else {
            theme::inactive()
        })
    };
    let panes_open = !app.visible_panes().is_empty();
    let keys_hint = Paragraph::new(Line::from(vec![
        Span::styled(" | ", Style::default().fg(theme::border())),
        Span::styled("P", key_style(app.show_projects)),
        Span::styled("/", Style::default().fg(theme::border())),
        Span::styled("T", key_style(app.show_tags)),
        Span::styled("/", Style::default().fg(theme::border())),
        Span::styled("A", key_style(app.show_marks)),
        Span::styled(" | ", Style::default().fg(theme::border())),
        Span::styled("Tab", key_style(panes_open)),
        Span::styled(" | ", Style::default().fg(theme::border())),
        Span::styled("?", Style::default().fg(theme::accent())),
        Span::styled(": help", Style::default().fg(theme::inactive())),
    ]));
    f.render_widget(keys_hint, footer_chunks[1]);

    match app.input_mode {
        InputMode::Help => render_help_popup(f, app),
        InputMode::Detail => render_detail_popup(f, app),
        InputMode::Confirm => render_confirm_popup(f, app),
        InputMode::Onboarding => render_onboarding_popup(f, app),
        _ => {}
    }
}
