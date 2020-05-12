use chrono::{Local, SecondsFormat};

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;
use std::time::{Duration, Instant};

use crate::Verbosity;

lazy_static::lazy_static! {
    static ref LAST_UPDATE_PRINT_TIME: Mutex<Option<Instant>> = Mutex::new(None);
}

static LAST_PRINT_WAS_UPDATE: AtomicBool = AtomicBool::new(false);
static VERBOSE: AtomicBool = AtomicBool::new(false);
static QUIET: AtomicBool = AtomicBool::new(false);

pub fn set_verbosity(verbosity: &Verbosity) {
    VERBOSE.store(verbosity.verbose, Ordering::Relaxed);
    QUIET.store(verbosity.quiet, Ordering::Relaxed);
}

pub fn println(s: &str) {
    if !QUIET.load(Ordering::Relaxed) {
        eprintln!("[{}] {}", timestamp(), s);
    }
}

pub fn verbose_println(s: &str, updating_line: bool) {
    if VERBOSE.load(Ordering::Relaxed) {
        let width = term_size::dimensions().map_or(70, |(w, _)| w - 1).max(4);

        if updating_line {
            // Throttle the line updates to once per 20ms, 50 Hz is plenty real-time.
            if let Ok(mut last_update) = LAST_UPDATE_PRINT_TIME.lock() {
                let now = Instant::now();
                if last_update.is_some() && now - last_update.unwrap() < Duration::from_millis(20) {
                    return;
                }
                *last_update = Some(now);
            }

            // Clear the line, then write the line, but limit it to the terminal width
            if s.len() >= width - 1 {
                eprint!("{:width$}\r{}...\r", "", &s[..width - 4], width = width);
            } else {
                eprint!("{:width$}\r{}\r", "", s, width = width);
            };
            LAST_PRINT_WAS_UPDATE.store(true, Ordering::Relaxed);
        } else {
            let was_update = LAST_PRINT_WAS_UPDATE.swap(false, Ordering::Relaxed);
            if was_update {
                // Clear the line
                eprint!("{:width$}\r", "", width = width);
                if let Ok(mut last_update) = LAST_UPDATE_PRINT_TIME.lock() {
                    *last_update = None;
                }
            }
            eprintln!("[{}] {}", timestamp(), s);
        }
    }
}

fn timestamp() -> String {
    Local::now().to_rfc3339_opts(SecondsFormat::Secs, true)
}
