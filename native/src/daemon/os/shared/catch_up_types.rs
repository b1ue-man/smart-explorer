//! Public catch-up results shared by the desktop and Android hosts.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CatchUpSkip {
    pub job_id: String,
    pub job_name: String,
    pub reason: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CatchUpStatus {
    pub finished: bool,
    /// Display name of this run's job that is running right now.
    pub running_job: Option<String>,
    /// This run's admitted jobs still waiting in the supervisor queue.
    pub queued: usize,
    pub message: Option<String>,
    /// Jobs admitted to or awaited by this run (done = admitted - queued -
    /// running).
    pub admitted: usize,
    /// Selected jobs the supervisor refused, with the reason.
    pub skipped: Vec<CatchUpSkip>,
    /// Jobs of this run whose attempt failed.
    pub failed: usize,
    /// At least one failure is transient (side unreachable, command, file
    /// errors): the host should retry its window (WorkManager
    /// `Result.retry()`). Login, configuration and access failures and
    /// blocks do not ask for a retry.
    pub retry_suggested: bool,
}

/// The last finished catch-up run that ran at least one job, with its
/// outcome (kept across restarts).
#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct CatchUpRecord {
    pub finished_ms: i64,
    /// Jobs that ran (admitted and finished, not skipped).
    pub ran: usize,
    pub succeeded: usize,
    pub failed: usize,
    pub message: String,
}
