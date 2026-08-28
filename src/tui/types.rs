#[derive(Clone, Copy, PartialEq, Debug)]
pub enum ViewMode {
    All,
    Day,
    Week,
    Overview,
}

impl ViewMode {
    pub fn title(&self) -> &'static str {
        match self {
            ViewMode::All => "All Entries",
            ViewMode::Day => "Daily View",
            ViewMode::Week => "Weekly View",
            ViewMode::Overview => "Overview",
        }
    }

    /// The one-word scope name, for naming the period inline: `day · all projects`.
    pub fn label(&self) -> &'static str {
        match self {
            ViewMode::All => "all",
            ViewMode::Day => "day",
            ViewMode::Week => "week",
            ViewMode::Overview => "year",
        }
    }
}

#[derive(Clone, Copy, PartialEq, Debug)]
pub enum InputMode {
    Normal,
    AddingEntry,
    EditingEntry,
    Searching,
    Help,
    /// The selected entry's detail popover. Modal, but the list stays live
    /// underneath: j/k move the table cursor and the popover re-reads the selection.
    Detail,
    /// A destructive action waiting for a yes; `App.pending_confirm` says which.
    Confirm,
    /// First-run popup asking which collapsible surfaces should start open.
    Onboarding,
}

/// Which destructive action a prompt is standing in front of. Each knows the key
/// that raised it, since that same key is what confirms it.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum ConfirmAction {
    Delete,
    Trim,
}

impl ConfirmAction {
    /// The key that opens this prompt, and therefore also confirms it.
    pub fn key(self) -> char {
        match self {
            ConfirmAction::Delete => 'd',
            ConfirmAction::Trim => 't',
        }
    }

    /// The yes keys as the hint row spells them, originating key first.
    pub fn confirm_keys(self) -> &'static str {
        match self {
            ConfirmAction::Delete => "d / y",
            ConfirmAction::Trim => "t / y",
        }
    }

    /// The prompt's verb, capitalised. **`Trim`, never "split"** — the store
    /// mechanic is `split_at_idle`, the user-facing verb is trim on every surface.
    pub fn verb(self) -> &'static str {
        match self {
            ConfirmAction::Delete => "Delete",
            ConfirmAction::Trim => "Trim",
        }
    }
}

/// A destructive action asked for and not yet answered. **`entry_id` is captured
/// when the prompt is raised, not read back when it is answered** — the 250 ms poll
/// can move the table cursor while the prompt is on screen.
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct PendingConfirm {
    pub action: ConfirmAction,
    pub entry_id: u64,
    /// The mode to restore on cancel.
    pub from: InputMode,
}

#[derive(Clone, Copy, PartialEq, Debug)]
pub enum InputField {
    Description,
    Project,
    Tags,
    StartTime,
    EndTime,
    Duration,
}

/// One of the two collapsible value pickers above the tabs row.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Pane {
    Projects,
    Tags,
}

impl Pane {
    /// Block title, with the toggle key so the surface documents itself.
    pub fn title(self) -> &'static str {
        match self {
            Pane::Projects => " Projects (P) ",
            Pane::Tags => " Tags (T) ",
        }
    }
}

/// Which screen `InputMode::Onboarding` is showing. Onboarding is linear:
/// `Layout` picks panel defaults, then falls through to `Skill`.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum OnboardingStep {
    Layout,
    Skill,
}

/// One row of the onboarding popup's checklist: a collapsible surface plus the
/// key that toggles it in normal use, so the popup can teach that key.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum LayoutSurface {
    Projects,
    Agents,
    Summary,
    Tags,
}

impl LayoutSurface {
    pub const ALL: [LayoutSurface; 4] = [
        LayoutSurface::Projects,
        LayoutSurface::Agents,
        LayoutSurface::Summary,
        LayoutSurface::Tags,
    ];

    pub fn label(self) -> &'static str {
        match self {
            LayoutSurface::Projects => "Projects",
            LayoutSurface::Agents => "Agent",
            LayoutSurface::Summary => "Summary",
            LayoutSurface::Tags => "Tags",
        }
    }

    /// The key that toggles this surface in `InputMode::Normal`.
    pub fn key(self) -> char {
        match self {
            LayoutSurface::Projects => 'P',
            LayoutSurface::Agents => 'A',
            LayoutSurface::Summary => 'S',
            LayoutSurface::Tags => 'T',
        }
    }
}

/// What `j`/`k` move in `InputMode::Normal`; `Tab` cycles through it.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Focus {
    Table,
    Pane(Pane),
}

#[derive(Clone, Copy, PartialEq, Debug)]
pub enum SortOrder {
    NewestFirst,
    OldestFirst,
}

impl SortOrder {
    pub fn toggle(self) -> Self {
        match self {
            SortOrder::NewestFirst => SortOrder::OldestFirst,
            SortOrder::OldestFirst => SortOrder::NewestFirst,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            SortOrder::NewestFirst => "newest first",
            SortOrder::OldestFirst => "oldest first",
        }
    }
}
