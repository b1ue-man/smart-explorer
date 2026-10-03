use super::*;

impl App {
    /// Fresh configuration, shared JobState/RunSettings and cancellable worker.
    pub(in crate::app) fn run_job(&mut self, id: &str) {
        self.start_saved_desktop_run(id, None);
    }

    /// The existing update entrypoint remains; saved runs own their preparation.
    pub(in crate::app) fn drain_job_connect(&mut self) {}
}
