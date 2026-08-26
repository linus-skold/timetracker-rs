use super::overlay::{overlay_hints, render_overlay, wrap};
use crate::tui::types::{LayoutSurface, OnboardingStep};
use crate::tui::{App, theme};
use ratatui::{prelude::*, widgets::Paragraph};

/// First-run popup, one screen per `OnboardingStep`. A one-time question —
/// the toggle keys keep working afterwards regardless of what is picked here.
pub(super) fn render_onboarding_popup(f: &mut Frame, app: &App) {
    match app.onboarding_step {
        OnboardingStep::Layout => render_onboarding_layout_step(f, app),
        OnboardingStep::Skill => render_onboarding_skill_step(f, app),
    }
}

/// A short blurb per surface, so the checklist explains itself instead of
/// just naming things — and a glyph, purely decorative, to break up the list.
fn surface_blurb(surface: LayoutSurface) -> (&'static str, &'static str) {
    match surface {
        LayoutSurface::Projects => ("◆", "distinct projects in view, with counts"),
        LayoutSurface::Agents => ("◆", "open `tt agent` phases, live"),
        LayoutSurface::Summary => ("◆", "time split by project, this scope"),
        LayoutSurface::Tags => ("◆", "distinct tags in view, with counts"),
    }
}

/// Two dots per step, filled up to `current` (0-based) of `total`: a small
/// progress read at a glance, independent of the title's "(n/m)".
fn step_dots(current: usize, total: usize) -> Line<'static> {
    let mut spans = vec![Span::raw("  ")];
    for i in 0..total {
        if i > 0 {
            spans.push(Span::styled("─", Style::default().fg(theme::border())));
        }
        spans.push(Span::styled(
            "●",
            if i <= current {
                Style::default().fg(theme::accent())
            } else {
                Style::default().fg(theme::border())
            },
        ));
    }
    Line::from(spans)
}

fn render_onboarding_layout_step(f: &mut Frame, app: &App) {
    let mut lines: Vec<Line> = vec![
        Line::from(Span::styled(
            "  Welcome to tt.",
            Style::default().fg(theme::highlight()).bold(),
        )),
        Line::from(Span::styled(
            "  A couple of quick questions, then you're set.",
            Style::default().fg(theme::inactive()).italic(),
        )),
        Line::from(Span::raw("")),
        Line::from(Span::styled(
            "  Which panels should start open?",
            Style::default().fg(theme::title()),
        )),
        Line::from(Span::raw("")),
    ];
    for (i, surface) in LayoutSurface::ALL.iter().enumerate() {
        let cursor = if i == app.onboarding_cursor {
            " ▸ "
        } else {
            "   "
        };
        let checked = app.onboarding_is_checked(*surface);
        let box_glyph = if checked { "◉" } else { "○" };
        let (glyph, blurb) = surface_blurb(*surface);
        let row_style = if i == app.onboarding_cursor {
            Style::default().fg(theme::highlight()).bold()
        } else if checked {
            Style::default().fg(theme::active())
        } else {
            Style::default().fg(theme::inactive())
        };
        lines.push(Line::from(vec![
            Span::styled(cursor, Style::default().fg(theme::accent()).bold()),
            Span::styled(
                format!("{box_glyph} {} {glyph} ", surface.label()),
                row_style,
            ),
            Span::styled(blurb, Style::default().fg(theme::inactive())),
        ]));
        lines.push(Line::from(vec![
            Span::raw("      "),
            Span::styled(
                format!("toggle later with Shift-{}", surface.key()),
                Style::default().fg(theme::border()),
            ),
        ]));
    }
    lines.push(Line::from(Span::raw("")));
    lines.push(step_dots(0, 2));

    let content = render_overlay(
        f,
        58,
        lines.len() as u16 + 3,
        Span::styled(
            " Welcome (1/2) ",
            Style::default().fg(theme::highlight()).bold(),
        ),
        Span::styled(" setup ", Style::default().fg(theme::inactive())),
        overlay_hints(&[
            ("j/k", "move"),
            ("space", "toggle"),
            ("s", "next"),
            ("esc", "skip"),
        ]),
    );
    f.render_widget(
        Paragraph::new(lines).style(Style::default().bg(theme::OVERLAY_BG)),
        content,
    );
}

/// Offers to install the `AGENTS.md` time-logging contract as a skill.
/// `y` hands the terminal to the child process; `n`/`Enter` moves on.
fn render_onboarding_skill_step(f: &mut Frame, app: &App) {
    let mut lines: Vec<Line> = vec![
        Line::from(Span::styled(
            format!("  {} One last thing.", crate::icons::agent()),
            Style::default().fg(theme::highlight()).bold(),
        )),
        Line::from(Span::raw("")),
        Line::from(Span::styled(
            "  Teach your coding agent the tt workflow —",
            Style::default().fg(theme::title()),
        )),
        Line::from(Span::styled(
            "  `agent begin` / `touch` / `end`, phase timing",
            Style::default().fg(theme::title()),
        )),
        Line::from(Span::styled(
            "  that survives a compacted context.",
            Style::default().fg(theme::title()),
        )),
        Line::from(Span::raw("")),
        Line::from(vec![
            Span::styled("    ", Style::default()),
            Span::styled("$ ", Style::default().fg(theme::border())),
            Span::styled(
                "npx skills add linus-skold/timetracker-rs",
                Style::default().fg(theme::accent()).bold(),
            ),
        ]),
        Line::from(Span::raw("")),
    ];

    // `y`'s existence check failed last attempt: say so in place, rather than
    // suspending the terminal to run a command already known to be missing.
    if let Some(error) = &app.onboarding_skill_error {
        lines.push(Line::from(Span::styled(
            format!("  {} ", crate::icons::warning()),
            Style::default(),
        )));
        for (i, line) in wrap(error, 46).into_iter().enumerate() {
            let prefix = if i == 0 { "  " } else { "    " };
            lines.push(Line::from(Span::styled(
                format!("{prefix}{line}"),
                Style::default().fg(theme::theme().duration_high),
            )));
        }
        lines.push(Line::from(Span::raw("")));
    }
    lines.push(step_dots(1, 2));

    let content = render_overlay(
        f,
        58,
        lines.len() as u16 + 3,
        Span::styled(
            " Welcome (2/2) ",
            Style::default().fg(theme::highlight()).bold(),
        ),
        Span::styled(" setup ", Style::default().fg(theme::inactive())),
        overlay_hints(&[("y", "install"), ("n / enter", "skip"), ("esc", "skip all")]),
    );
    f.render_widget(
        Paragraph::new(lines).style(Style::default().bg(theme::OVERLAY_BG)),
        content,
    );
}
