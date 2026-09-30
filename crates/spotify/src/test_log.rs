// Captures `log` records per thread. Tests run on their own threads, so each sees only its own lines.

use std::cell::RefCell;
use std::sync::Once;

use log::{Level, LevelFilter, Log, Metadata, Record};

thread_local! {
    static LINES: RefCell<Option<Vec<String>>> = const { RefCell::new(None) };
}

struct Capture;

impl Log for Capture {
    fn enabled(&self, metadata: &Metadata) -> bool {
        metadata.level() <= Level::Warn
    }

    fn log(&self, record: &Record) {
        LINES.with(|l| {
            if let Some(lines) = l.borrow_mut().as_mut() {
                lines.push(record.args().to_string());
            }
        });
    }

    fn flush(&self) {}
}

/// Runs `f` and returns every warning it logged on this thread.
pub fn capture(f: impl FnOnce()) -> Vec<String> {
    static INIT: Once = Once::new();
    INIT.call_once(|| {
        log::set_logger(&Capture).unwrap();
        log::set_max_level(LevelFilter::Warn);
    });
    LINES.with(|l| *l.borrow_mut() = Some(Vec::new()));
    f();
    LINES.with(|l| l.borrow_mut().take().unwrap_or_default())
}
