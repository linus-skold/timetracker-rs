//! The first-run popup: a checklist for which collapsible surfaces start
//! open. Never forecloses anything — every surface keeps its own toggle key.

use anyhow::Result;

use super::App;
use super::types::{Focus, InputMode, LayoutSurface, OnboardingStep};
use crate::config::LayoutConfig;

impl App {
    pub(crate) fn onboarding_move(&mut self, delta: isize) {
        let len = LayoutSurface::ALL.len() as isize;
        let next = (self.onboarding_cursor as isize + delta).rem_euclid(len);
        self.onboarding_cursor = next as usize;
    }

    pub(crate) fn onboarding_toggle(&mut self) {
        let checked = &mut self.onboarding_checked[self.onboarding_cursor];
        *checked = !*checked;
    }

    pub(crate) fn onboarding_is_checked(&self, surface: LayoutSurface) -> bool {
        let idx = LayoutSurface::ALL
            .iter()
            .position(|s| *s == surface)
            .unwrap();
        self.onboarding_checked[idx]
    }

    /// `s` on the layout step: applies the checklist live and moves on to the
    /// skill step. Not yet persisted — that's `onboarding_finish`/`_skip`.
    pub(crate) fn onboarding_apply_layout(&mut self) {
        let [projects, agents, summary, tags] = self.onboarding_checked;
        self.show_projects = projects;
        self.show_marks = agents;
        self.show_summary = summary;
        self.show_tags = tags;
        // Mirrors `toggle_pane`: opening a panel also focuses it.
        if let Some(first) = self.visible_panes().first() {
            self.focus = Focus::Pane(*first);
        }
        self.onboarding_step = OnboardingStep::Skill;
    }

    /// Persists the checklist answer and returns to normal use.
    pub(crate) fn onboarding_finish(&mut self) -> Result<()> {
        let [projects, agents, summary, tags] = self.onboarding_checked;
        crate::config::save_onboarding(&LayoutConfig {
            show_projects: Some(projects),
            show_agents: Some(agents),
            show_summary: Some(summary),
            show_tags: Some(tags),
        })?;
        self.input_mode = InputMode::Normal;
        Ok(())
    }

    /// `Esc`: a cancel, not "close everything" — persists the panels exactly
    /// as they already were, and marks onboarding done so it stays quiet.
    pub(crate) fn onboarding_skip(&mut self) -> Result<()> {
        crate::config::save_onboarding(&LayoutConfig {
            show_projects: Some(self.show_projects),
            show_agents: Some(self.show_marks),
            show_summary: Some(self.show_summary),
            show_tags: Some(self.show_tags),
        })?;
        self.input_mode = InputMode::Normal;
        Ok(())
    }
}
