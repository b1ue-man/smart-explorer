//! Host-provided URL opener. An embedding host (the Android app) registers a
//! callback that shows a URL in its browser; while none is registered the
//! platform adapter opens it. The desktop builds never register one, so their
//! behavior stays the platform adapter's.
use std::sync::{Arc, RwLock};

type UrlOpener = Arc<dyn Fn(&str) + Send + Sync>;

static OPENER: RwLock<Option<UrlOpener>> = RwLock::new(None);

/// Registers the host's URL opener; a later registration replaces it.
pub fn set_url_opener(opener: Box<dyn Fn(&str) + Send + Sync>) {
    let mut slot = OPENER
        .write()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    *slot = Some(Arc::from(opener));
}

/// Hands `url` to the registered host opener. `false` when none is registered.
pub(super) fn open_with_host(url: &str) -> bool {
    // Clone the handle out so the callback never runs under the lock.
    let opener = OPENER
        .read()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .clone();
    match opener {
        Some(open) => {
            open(url);
            true
        }
        None => false,
    }
}
