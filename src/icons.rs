/// Icons and emoji used throughout the application, overridable via the
/// `[icons]` table in the user's config file.
use std::sync::OnceLock;

use crate::config;

struct Icons {
    active: String,
    stopped: String,
    logged: String,
    warning: String,
    calendar: String,
    list: String,
    agent: String,
}

impl Icons {
    fn from_config(cfg: &config::IconsConfig) -> Self {
        let icon = |s: &Option<String>, default: &str| s.clone().unwrap_or_else(|| default.to_string());

        Icons {
            active: icon(&cfg.active, "▶️"),
            stopped: icon(&cfg.stopped, "⏹️"),
            logged: icon(&cfg.logged, "📝"),
            warning: icon(&cfg.warning, "⚠️"),
            calendar: icon(&cfg.calendar, "📅"),
            list: icon(&cfg.list, "📋"),
            agent: icon(&cfg.agent, "🤖"),
        }
    }
}

static ICONS: OnceLock<Icons> = OnceLock::new();

fn icons() -> &'static Icons {
    ICONS.get_or_init(|| Icons::from_config(&config::load().icons))
}

pub fn active() -> &'static str {
    &icons().active
}
pub fn stopped() -> &'static str {
    &icons().stopped
}
pub fn logged() -> &'static str {
    &icons().logged
}
pub fn warning() -> &'static str {
    &icons().warning
}
pub fn calendar() -> &'static str {
    &icons().calendar
}
pub fn list() -> &'static str {
    &icons().list
}
pub fn agent() -> &'static str {
    &icons().agent
}
