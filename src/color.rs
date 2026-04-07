use std::io::IsTerminal;
use std::sync::atomic::{AtomicBool, Ordering};

static COLOR_ENABLED: AtomicBool = AtomicBool::new(false);

/// Initialize color support. Call once at startup.
pub fn init() {
    COLOR_ENABLED.store(std::io::stderr().is_terminal(), Ordering::Relaxed);
}

fn enabled() -> bool {
    COLOR_ENABLED.load(Ordering::Relaxed)
}

pub fn green(s: &str) -> String {
    if enabled() { format!("\x1b[32m{s}\x1b[0m") } else { s.to_string() }
}

pub fn yellow(s: &str) -> String {
    if enabled() { format!("\x1b[33m{s}\x1b[0m") } else { s.to_string() }
}

pub fn red(s: &str) -> String {
    if enabled() { format!("\x1b[31m{s}\x1b[0m") } else { s.to_string() }
}

pub fn cyan(s: &str) -> String {
    if enabled() { format!("\x1b[36m{s}\x1b[0m") } else { s.to_string() }
}

pub fn dim(s: &str) -> String {
    if enabled() { format!("\x1b[2m{s}\x1b[0m") } else { s.to_string() }
}

pub fn bold(s: &str) -> String {
    if enabled() { format!("\x1b[1m{s}\x1b[0m") } else { s.to_string() }
}
