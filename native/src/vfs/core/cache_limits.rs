#[derive(Clone, Copy)]
pub(super) struct CacheLimits {
    pub(super) directories: usize,
    pub(super) entries: usize,
    pub(super) bytes: usize,
}

impl CacheLimits {
    pub(super) const BROWSING: Self = Self {
        directories: 4_096,
        entries: 50_000,
        bytes: 32 * 1024 * 1024,
    };
    // Mount limits govern retention, never directory validity or traversal.
    pub(super) const MOUNT: Self = Self {
        directories: usize::MAX,
        entries: usize::MAX,
        bytes: 64 * 1024 * 1024,
    };
}
