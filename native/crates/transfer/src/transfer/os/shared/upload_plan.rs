//! Destination-name reservation for single streamed uploads.
use crate::vfs::remote_util::{numbered_remote_name, rjoin, REMOTE_UNIQUE_ATTEMPTS};
use std::collections::HashSet;
use std::sync::atomic::AtomicBool;

pub(super) struct DestinationNames {
    reserved: HashSet<String>,
    case_sensitive: bool,
}

impl DestinationNames {
    pub(super) fn new(backend: &dyn crate::vfs::Backend, root: &str) -> Self {
        Self {
            reserved: HashSet::new(),
            case_sensitive: backend.case_sensitive_paths(root),
        }
    }

    pub(super) fn reserve(
        &mut self,
        backend: &dyn crate::vfs::Backend,
        parent: &str,
        base: &str,
        cancel: &AtomicBool,
    ) -> Result<String, String> {
        for index in 1..=REMOTE_UNIQUE_ATTEMPTS {
            super::cancel::check(cancel)?;
            let name = numbered_remote_name(base, index);
            let path = rjoin(parent, &name);
            // Conservative reservation when a provider cannot promise distinct
            // case variants. The provider's non-replacing copy publication
            // remains responsible for aliases and concurrent creators; ID-based
            // providers do not promise atomic sibling-name reservations.
            let key = if self.case_sensitive {
                path.clone()
            } else {
                path.to_uppercase()
            };
            if self.reserved.contains(&key) {
                continue;
            }
            let exists = backend.try_exists(&path);
            super::cancel::check(cancel)?;
            match exists {
                Ok(false) => {
                    self.reserved.insert(key);
                    return Ok(name);
                }
                Ok(true) => {}
                Err(error) => return Err(format!("Ziel prüfen „{path}“: {error}")),
            }
        }
        Err(format!(
            "Kein freier Name nach {} Versuchen",
            REMOTE_UNIQUE_ATTEMPTS
        ))
    }
}
