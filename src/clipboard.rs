//! The clipboard: reading it for paste, and watching it for copied links.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

/// The clipboard's text, if any.
pub fn text() -> Option<String> {
    arboard::Clipboard::new().ok()?.get_text().ok().filter(|t| !t.is_empty())
}

/// Calls `changed` with the clipboard's text whenever it changes, while
/// `on` is set (checked about once a second, on its own thread). What is
/// there at the start doesn't count as a change.
pub fn watch(on: Arc<AtomicBool>, changed: impl Fn(String) + Send + 'static) {
    let _ = std::thread::Builder::new().name("clipboard".into()).spawn(move || {
        let Ok(mut cb) = arboard::Clipboard::new() else { return };
        let mut last = cb.get_text().ok();
        loop {
            std::thread::sleep(Duration::from_millis(900));
            if !on.load(Ordering::Relaxed) {
                // Copies while off don't pop up when it's turned on again.
                last = cb.get_text().ok();
                continue;
            }
            let now = cb.get_text().ok();
            if now != last {
                last = now.clone();
                if let Some(t) = now.filter(|t| !t.is_empty()) {
                    changed(t);
                }
            }
        }
    });
}
