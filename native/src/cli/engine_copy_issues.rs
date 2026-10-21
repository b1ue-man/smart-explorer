//! The issues of a finished engine run: the first hundred come with the
//! result, the log file holds all of them (U49).
use super::Finished;
use crate::transfer::TransferIssue;

/// Every issue of the run: the engine carries the first hundred, the log file
/// all of them.
pub(super) struct Issues {
    pub(super) items: Vec<TransferIssue>,
    /// Issues counted but neither shown nor readable from the log.
    pub(super) unseen: u64,
}

pub(super) fn all_issues(finished: &Finished) -> Issues {
    let total = finished.progress.errors;
    let shown = finished.issues.len() as u64;
    if total > shown {
        let logged = finished
            .progress
            .log_path
            .as_deref()
            .and_then(|path| std::fs::read_to_string(path).ok())
            .map(|text| parse_log(&text))
            .unwrap_or_default();
        if logged.len() as u64 >= total {
            return Issues {
                items: logged,
                unseen: 0,
            };
        }
        return Issues {
            items: finished.issues.clone(),
            unseen: total - shown,
        };
    }
    Issues {
        items: finished.issues.clone(),
        unseen: 0,
    }
}

/// The issue log: one JSON object per line. A sealed line (vault) has no
/// readable fields and counts as a failure with a fixed text.
pub(in crate::cli) fn parse_log(text: &str) -> Vec<TransferIssue> {
    text.lines()
        .filter_map(|line| serde_json::from_str::<serde_json::Value>(line).ok())
        .map(|value| {
            let field = |name: &str| value.get(name).and_then(|v| v.as_str()).map(str::to_string);
            match (field("path"), field("message")) {
                (Some(path), Some(message)) => TransferIssue { path, message },
                _ => TransferIssue {
                    path: String::new(),
                    message: "details of a vault entry are sealed in the log".to_string(),
                },
            }
        })
        .collect()
}
