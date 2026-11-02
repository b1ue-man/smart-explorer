//! One classification of a finished engine run for every runner (background
//! worker, desktop window, Android facade), so the job state counts runs the
//! same way everywhere (RV1, contract V4), and the translation between the
//! engine's safety stops (V3 `RunBlock`) and the job state's `BlockKind`.

use crate::bisync::{BlockConfirmation, Outcome, PairSide, RunBlock, RunStop};

use super::job_state::{AttemptOutcome, BlockKind, Blocked, FailureKind, JobError, JobSide};
use super::results::JobResult;

/// Error kind with which older engine versions reported a stop they decided
/// themselves (delete guard); kept as a fallback for such outcomes.
const ENGINE_STOP: &str = "abgebrochen";

/// Classifies a finished `bisync::run`/`run_with`. `canceled` = the runner's
/// own cancel flag was set (user, pause, worker stop, host). A safety stop is
/// a block, never a cancellation, so it is not repeated every minute; a pair
/// held by another runner (`busy`) counts nothing, like a cancellation, and
/// keeps the trigger. Errors count `stats.errors`, because the message list is
/// capped.
pub fn classify_run(out: &Outcome, canceled: bool, finished: i64) -> (AttemptOutcome, JobResult) {
    let legacy_stop = out.errors.iter().find(|(kind, _)| kind == ENGINE_STOP);
    let errors = out
        .stats
        .errors
        .max(u64::try_from(out.errors.len()).unwrap_or(u64::MAX));
    let canceled = canceled || out.canceled;
    let outcome = if canceled || out.busy {
        AttemptOutcome::Cancelled
    } else if let Some(block) = &out.blocked {
        AttemptOutcome::Blocked(Blocked {
            kind: block_kind(block),
            detail: block.message(),
            since: finished,
            confirmed: false,
        })
    } else if let Some((_, detail)) = legacy_stop {
        AttemptOutcome::Blocked(Blocked {
            kind: BlockKind::Other,
            detail: detail.clone(),
            since: finished,
            confirmed: false,
        })
    } else if let Some(error) = out.errors.iter().find(|(_, message)| {
        matches!(
            classify_failure(message),
            FailureKind::Auth | FailureKind::Access
        )
    }) {
        AttemptOutcome::Failed(JobError {
            kind: classify_failure(&error.1),
            message: format!("{}: {}", error.0, error.1),
        })
    } else if let Some(stop) = out.stopped {
        AttemptOutcome::Failed(JobError {
            kind: stop_kind(stop),
            message: stop.message(),
        })
    } else if errors > 0 {
        AttemptOutcome::Failed(JobError {
            kind: FailureKind::Run,
            message: run_error_message(out, errors),
        })
    } else {
        AttemptOutcome::Success
    };
    let status = match &outcome {
        AttemptOutcome::Cancelled if out.busy && !canceled => "läuft bereits",
        AttemptOutcome::Cancelled | AttemptOutcome::Blocked(_) => "abgebrochen",
        AttemptOutcome::Failed(_) => "Fehler",
        AttemptOutcome::Success if !out.conflicts.is_empty() => "Konflikte",
        AttemptOutcome::Success => "ok",
    };
    let result = JobResult {
        when: finished,
        a_to_b: out.stats.a_to_b,
        b_to_a: out.stats.b_to_a,
        deleted: out.stats.deleted,
        conflicts: u64::try_from(out.conflicts.len()).unwrap_or(u64::MAX),
        errors,
        note: out.omissions.result_note(status),
    };
    (outcome, result)
}

fn run_error_message(out: &Outcome, errors: u64) -> String {
    match out.errors.first() {
        Some((path, message)) if errors == 1 => format!("{path}: {message}"),
        Some((path, message)) => format!("{errors} Fehler, zuerst {path}: {message}"),
        None => format!("{errors} Fehler"),
    }
}

fn stop_kind(stop: RunStop) -> FailureKind {
    match stop {
        RunStop::TargetFull { .. } | RunStop::TargetReadOnly { .. } => FailureKind::TargetFull,
        RunStop::ConnectionLost { .. } => FailureKind::Unreachable,
    }
}

fn job_side(side: PairSide) -> JobSide {
    match side {
        PairSide::A => JobSide::A,
        PairSide::B => JobSide::B,
    }
}

fn pair_side(side: JobSide) -> PairSide {
    match side {
        JobSide::A => PairSide::A,
        JobSide::B => PairSide::B,
    }
}

/// The job state's form of an engine stop.
pub fn block_kind(block: &RunBlock) -> BlockKind {
    match block {
        RunBlock::MassDelete {
            side,
            deletes,
            files,
        } => BlockKind::MassDelete {
            side: job_side(*side),
            deletions: *deletes,
            total: *files,
        },
        RunBlock::DeleteLimit { deletes, limit } => BlockKind::DeleteLimit {
            deletions: *deletes,
            limit: *limit,
        },
        RunBlock::SideEmpty { side, previous } => BlockKind::SideEmpty {
            side: job_side(*side),
            previous: *previous,
        },
        RunBlock::ReplicaMissing { side } => BlockKind::ReplicaMissing {
            side: job_side(*side),
        },
    }
}

/// The engine confirmation that lets exactly this stop pass once ("Trotzdem
/// ausführen"); `None` for a stop without details (`Other`).
pub fn block_confirmation(kind: &BlockKind) -> Option<BlockConfirmation> {
    let block = match kind {
        BlockKind::MassDelete {
            side,
            deletions,
            total,
        } => RunBlock::MassDelete {
            side: pair_side(*side),
            deletes: *deletions,
            files: *total,
        },
        BlockKind::DeleteLimit { deletions, limit } => RunBlock::DeleteLimit {
            deletes: *deletions,
            limit: *limit,
        },
        BlockKind::SideEmpty { side, previous } => RunBlock::SideEmpty {
            side: pair_side(*side),
            previous: *previous,
        },
        BlockKind::ReplicaMissing { side } => RunBlock::ReplicaMissing {
            side: pair_side(*side),
        },
        BlockKind::Other => return None,
    };
    Some(block.confirmation())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn review_task_classification_follows_the_engine_outcome() {
        let finished = 1_000;
        let blocked = Outcome {
            blocked: Some(RunBlock::SideEmpty {
                side: PairSide::B,
                previous: 40,
            }),
            ..Default::default()
        };
        match classify_run(&blocked, false, finished).0 {
            AttemptOutcome::Blocked(block) => {
                assert_eq!(
                    block.kind,
                    BlockKind::SideEmpty {
                        side: JobSide::B,
                        previous: 40
                    }
                );
                assert!(block.detail.contains("leer"));
            }
            other => panic!("a safety stop is a block: {other:?}"),
        }
        let busy = Outcome {
            busy: true,
            ..Default::default()
        };
        assert_eq!(
            classify_run(&busy, false, finished).0,
            AttemptOutcome::Cancelled
        );
        let full = Outcome {
            stopped: Some(RunStop::TargetFull { side: PairSide::B }),
            ..Default::default()
        };
        assert!(matches!(
            classify_run(&full, false, finished).0,
            AttemptOutcome::Failed(JobError {
                kind: FailureKind::TargetFull,
                ..
            })
        ));
        let mut errors = Outcome::default();
        errors.stats.errors = 12_000;
        errors.errors = vec![("a.txt".into(), "kein Zugriff".into())];
        let (outcome, result) = classify_run(&errors, false, finished);
        assert!(matches!(outcome, AttemptOutcome::Failed(_)));
        assert_eq!(result.errors, 12_000, "the real count, not the capped list");
        assert_eq!(
            classify_run(&Outcome::default(), true, finished).0,
            AttemptOutcome::Cancelled
        );
        assert_eq!(
            classify_run(&Outcome::default(), false, finished).0,
            AttemptOutcome::Success
        );
    }

    #[test]
    fn review_task_block_confirmation_round_trips() {
        let block = RunBlock::MassDelete {
            side: PairSide::A,
            deletes: 120,
            files: 200,
        };
        let kind = block_kind(&block);
        assert_eq!(block_confirmation(&kind), Some(block.confirmation()));
        assert!(block_confirmation(&kind).is_some_and(|confirmation| confirmation.covers(&block)));
        assert_eq!(block_confirmation(&BlockKind::Other), None);
    }
}

/// Conservative classification until resolvers provide structured failures.
/// Authentication and permissions never cause automatic account probes.
pub fn classify_failure(message: &str) -> FailureKind {
    let lower = message.to_lowercase();
    // Operation labels describe where an error happened, not its cause. In
    // particular, a failed read of the credential store is not a rejected
    // login, and an invalid remote response is not a broken saved job.
    let detail = [
        "gespeicherte anmeldeinformation lesen: ",
        "oauth-verbindung dauerhaft speichern: ",
        "erneuertes oauth-token dauerhaft speichern: ",
        "drive account identity request failed: ",
        "drive account identity response failed: ",
    ]
    .iter()
    .find_map(|prefix| lower.split_once(*prefix).map(|(_, detail)| detail))
    .unwrap_or(lower.as_str());
    // Only a freshly confirmed principal mismatch needs reauthentication;
    // ordinary rotation or an unavailable about response remains retryable.
    if detail.trim_end().ends_with(
        "drive-konto wurde gewechselt; gespeicherte ordnerbindung gehört zu einem anderen konto",
    ) {
        return FailureKind::Auth;
    }
    if lower.contains("drive account identity response is invalid:")
        || lower.contains("drive account identity response has no permissionid")
        || transient_http_status(detail)
    {
        return FailureKind::Unreachable;
    }
    if detail.trim() == "nicht verbunden"
        || [
            "auth",
            "anmeld",
            "passwort",
            "password",
            "credential",
            "unauthorized",
            "401",
            "invalid_grant",
            "schlüssel",
            "key rejected",
            "login",
            "not logged in",
            "530 ",
            "host key",
            "certificate",
            "zertifikat",
        ]
        .iter()
        .any(|part| detail.contains(part))
    {
        FailureKind::Auth
    } else if ["permission", "zugriff", "access denied", "403"]
        .iter()
        .any(|part| detail.contains(part))
    {
        FailureKind::Access
    } else if [
        "ungültig",
        "invalid",
        "nicht unterstützt",
        "unsupported",
        "keine gespeicherte",
    ]
    .iter()
    .any(|part| detail.contains(part))
    {
        FailureKind::Config
    } else {
        FailureKind::Unreachable
    }
}

fn transient_http_status(detail: &str) -> bool {
    // Match an actual response status rather than arbitrary digits in a path
    // or authority; OAuth/request context must not turn a 5xx into Auth.
    ["status code ", "http status ", "http "]
        .iter()
        .filter_map(|prefix| detail.split_once(*prefix).map(|(_, tail)| tail))
        .any(|tail| {
            let code = tail.split(|ch: char| !ch.is_ascii_digit()).next();
            code.and_then(|code| code.parse::<u16>().ok())
                .is_some_and(|code| matches!(code, 408 | 429 | 500..=599))
        })
}
