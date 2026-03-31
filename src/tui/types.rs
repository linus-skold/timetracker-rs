#[derive(Clone, Copy, PartialEq)]
pub enum ViewMode {
    All,
    Day,
    Week,
}

impl ViewMode {
    pub fn title(&self) -> &'static str {
        match self {
            ViewMode::All => "All Entries",
            ViewMode::Day => "Daily View",
            ViewMode::Week => "Weekly View",
        }
    }
}

#[derive(Clone, Copy, PartialEq)]
pub enum InputMode {
    Normal,
    AddingEntry,
    EditingEntry,
    Searching,
    Help,
}

#[derive(Clone, Copy, PartialEq)]
pub enum InputField {
    Description,
    Tags,
    StartTime,
    EndTime,
    Duration,
}

#[derive(Clone, Copy, PartialEq)]
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
