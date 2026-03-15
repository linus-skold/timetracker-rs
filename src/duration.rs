use chrono::Duration;

/// Format a duration as "Xh Ym"
pub fn format(dur: Duration) -> String {
    let hours = dur.num_hours();
    let minutes = dur.num_minutes() % 60;
    format!("{}h {}m", hours, minutes)
}

/// Parse a duration string like "1h30m", "45m", "2h", or bare minutes "45"
pub fn parse(duration_str: &str) -> Duration {
    let mut hours = 0i64;
    let mut minutes = 0i64;
    let mut current_num = String::new();

    for c in duration_str.chars() {
        match c {
            '0'..='9' => current_num.push(c),
            'h' | 'H' => {
                hours = current_num.parse().unwrap_or(0);
                current_num.clear();
            }
            'm' | 'M' => {
                minutes = current_num.parse().unwrap_or(0);
                current_num.clear();
            }
            _ => {}
        }
    }

    // Handle bare number as minutes
    if !current_num.is_empty() && hours == 0 && minutes == 0 {
        minutes = current_num.parse().unwrap_or(0);
    }

    Duration::hours(hours) + Duration::minutes(minutes)
}
