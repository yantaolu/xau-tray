#[cfg(debug_assertions)]
pub fn log_line(message: &str) {
    println!("{message}");
}

#[cfg(not(debug_assertions))]
pub fn log_line(_message: &str) {}
