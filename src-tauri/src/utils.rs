use chrono::DateTime;

#[cfg(debug_assertions)]
pub fn log_line(message: &str) {
    println!("{message}");
}

#[cfg(not(debug_assertions))]
pub fn log_line(_message: &str) {}

pub fn parse_timestamp_rfc3339(value: &str) -> Option<u64> {
    DateTime::parse_from_rfc3339(value)
        .ok()
        .map(|dt| dt.timestamp() as u64)
}
