pub enum UpdateMsg {
    /// Feed reachable, no newer version. Only sent for manual checks.
    UpToDate { feed_version: String },
    /// No feed configured. Only sent for manual checks.
    NoFeed,
    /// In-place swap couldn't replace the running exe (locked); a detached
    /// worker was launched that will replace + relaunch after we exit. The app
    /// should just close; do not relaunch (the worker does).
    AppliedViaWorker { version: String },
    /// Only sent for manual checks; automatic checks fail silently.
    Error(String),
}
