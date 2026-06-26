use super::{IconKind, IconResult};

pub struct IconWorker;

impl IconWorker {
    pub fn new() -> Self {
        Self
    }

    pub fn request(&self, key: String, kind: IconKind) {
        let _ = (key, kind);
    }

    pub fn drain(&self) -> Vec<IconResult> {
        Vec::new()
    }
}
