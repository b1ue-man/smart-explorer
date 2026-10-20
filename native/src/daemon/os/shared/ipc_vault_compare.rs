//! The worker's side of the comparison requests of `IpcRequest::Vault`
//! (VLT-M8, U26-8): `Compare`, `CompareText`, `Resolve`, `MergeApply` and
//! `Check` run `crate::vault` in this process, where the keys are. A request
//! that takes long (a large vault over a network) watches its connection: a
//! client that closes it ends the work (`cancel`). Replies carry entry
//! numbers, cleartext paths and states for the caller who has the vault
//! unlocked, never a password; texts of a conflict travel as `SecretText`.
use std::io;
use std::net::TcpStream;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use crate::vault::{CompareGroup, ResolveReport, ResolveRequest};

use super::ipc_protocol::{
    write_response, CompareHead, CompareItemWire, IpcResponse, ResolveFailure, SecretText,
    VaultIpcReply,
};
use super::line::MAX_IPC_LINE;

/// How often the watch looks at the connection.
const WATCH_SLICE: Duration = Duration::from_millis(250);
/// Longest serialized JSON length of the entries of one page: half of the
/// line of the service (`MAX_IPC_LINE`); the rest is for the head and the
/// escaping.
pub(super) const ITEM_PAGE_BYTES: usize = MAX_IPC_LINE / 2;

fn failure(msg: impl Into<String>, code: Option<&str>) -> IpcResponse {
    IpcResponse::Err {
        msg: msg.into(),
        code: code.map(str::to_string),
    }
}

/// The reply for an error of a comparison request: a locked vault and a
/// comparison that is gone keep their codes.
fn error_response(error: &io::Error) -> IpcResponse {
    if crate::vault::locked_site(error).is_some() {
        return failure(error.to_string(), Some(crate::vault::VAULT_LOCKED_CODE));
    }
    if crate::vault::is_compare_stale(error) {
        return failure(error.to_string(), Some(crate::vault::COMPARE_STALE_CODE));
    }
    failure(error.to_string(), None)
}

fn watch_client(stream: &TcpStream, cancel: &AtomicBool, done: &AtomicBool) {
    let _ = stream.set_read_timeout(Some(WATCH_SLICE));
    let mut byte = [0u8; 1];
    while !done.load(Ordering::Acquire) {
        match stream.peek(&mut byte) {
            // The client closed the connection: nobody waits for the answer.
            Ok(0) => {
                cancel.store(true, Ordering::Release);
                return;
            }
            // More data than the request: nothing to read, look again later.
            Ok(_) => std::thread::sleep(WATCH_SLICE),
            Err(error)
                if matches!(
                    error.kind(),
                    io::ErrorKind::WouldBlock
                        | io::ErrorKind::TimedOut
                        | io::ErrorKind::Interrupted
                ) => {}
            Err(_) => {
                cancel.store(true, Ordering::Release);
                return;
            }
        }
    }
}

/// Runs `work` while a second thread watches the client's connection.
fn with_cancel<T>(stream: &TcpStream, work: impl FnOnce(&AtomicBool) -> T) -> T {
    let cancel = AtomicBool::new(false);
    let done = AtomicBool::new(false);
    let watch = stream.try_clone().ok();
    std::thread::scope(|scope| {
        if let Some(watch) = watch.as_ref() {
            scope.spawn(|| watch_client(watch, &cancel, &done));
        }
        let result = work(&cancel);
        done.store(true, Ordering::Release);
        result
    })
}

fn parse_groups(keys: &[String]) -> Result<Vec<CompareGroup>, String> {
    keys.iter()
        .map(|key| CompareGroup::from_key(key).ok_or_else(|| format!("Unbekannte Gruppe: {key}")))
        .collect()
}

/// The entries of `items` from `offset` on that fit one page, and the
/// offset of the rest (`None` at the end). A single entry always goes, so a
/// page is never empty.
pub(super) fn page_of(
    items: &[crate::vault::CompareItem],
    offset: usize,
) -> (Vec<CompareItemWire>, Option<u64>) {
    let mut page = Vec::new();
    let mut used = 0usize;
    let mut next = offset.min(items.len());
    for item in items.iter().skip(offset) {
        let entry = CompareItemWire::from(item);
        let size = serde_json::to_string(&entry).map_or(0, |text| text.len() + 1);
        if used + size > ITEM_PAGE_BYTES && !page.is_empty() {
            break;
        }
        used += size;
        page.push(entry);
        next += 1;
    }
    let rest = (next < items.len()).then(|| u64::try_from(next).unwrap_or(u64::MAX));
    (page, rest)
}

/// `response`, or the failure `compare_too_large` when it does not fit one
/// IPC line (the writer would drop the connection instead of answering).
fn fitting(response: IpcResponse) -> IpcResponse {
    let size = serde_json::to_string(&response).map_or(0, |text| text.len() + 1);
    if size > MAX_IPC_LINE {
        failure(
            crate::vault::error_text_for_code(crate::vault::COMPARE_TOO_LARGE_CODE)
                .unwrap_or("Die Datei ist zu groß für die Zusammenführung."),
            Some(crate::vault::COMPARE_TOO_LARGE_CODE),
        )
    } else {
        response
    }
}

pub(super) fn serve_compare(mut stream: TcpStream, job: &str, groups: &[String]) -> io::Result<()> {
    let response = match parse_groups(groups) {
        Err(message) => failure(message, None),
        Ok(groups) => {
            let view = with_cancel(&stream, |cancel| {
                crate::vault::compare_job(job, &groups, cancel)
            });
            match view {
                Ok(view) => IpcResponse::Vault(VaultIpcReply::Compared(CompareHead::from(&view))),
                Err(error) => error_response(&error),
            }
        }
    };
    write_response(&mut stream, &response)
}

/// One page of the entries of a comparison or a check.
pub(super) fn serve_items(
    mut stream: TcpStream,
    list: &str,
    revision: u64,
    offset: u64,
) -> io::Result<()> {
    let response = match crate::vault::compare_snapshot(list, revision) {
        Ok(items) => {
            let (items, next_offset) =
                page_of(&items, usize::try_from(offset).unwrap_or(usize::MAX));
            IpcResponse::Vault(VaultIpcReply::ItemPage { items, next_offset })
        }
        Err(error) => error_response(&error),
    };
    write_response(&mut stream, &response)
}

pub(super) fn serve_compare_text(mut stream: TcpStream, job: &str, item: &str) -> io::Result<()> {
    let response = match crate::vault::compare_text_job(job, item) {
        Ok((a, b)) => fitting(IpcResponse::Vault(VaultIpcReply::Texts {
            a: SecretText::new(a.as_str()),
            b: SecretText::new(b.as_str()),
        })),
        Err(error) => error_response(&error),
    };
    write_response(&mut stream, &response)
}

fn resolved(report: ResolveReport) -> IpcResponse {
    IpcResponse::Vault(VaultIpcReply::Resolved {
        item: report.item,
        result: "ok".to_string(),
        resolved: report.resolved,
        failed: report
            .failed
            .into_iter()
            .map(|(item, error)| ResolveFailure { item, error })
            .collect(),
    })
}

pub(super) fn serve_resolve(
    mut stream: TcpStream,
    job: &str,
    item: Option<&str>,
    action: &str,
    all: bool,
    confirmed: bool,
) -> io::Result<()> {
    let result = with_cancel(&stream, |cancel| {
        crate::vault::resolve_item(
            ResolveRequest {
                job,
                item,
                action,
                all,
                confirmed,
            },
            cancel,
        )
    });
    let response = match result {
        Ok(report) => resolved(report),
        Err(error) => error_response(&error),
    };
    write_response(&mut stream, &response)
}

pub(super) fn serve_merge_apply(
    mut stream: TcpStream,
    job: &str,
    item: &str,
    content: &SecretText,
) -> io::Result<()> {
    let result = with_cancel(&stream, |cancel| {
        crate::vault::merge_apply_item(job, item, content.expose(), cancel)
    });
    let response = match result {
        Ok(()) => resolved(ResolveReport {
            item: Some(item.to_string()),
            resolved: 1,
            failed: Vec::new(),
        }),
        Err(error) => error_response(&error),
    };
    write_response(&mut stream, &response)
}

pub(super) fn serve_check(mut stream: TcpStream, site: &str) -> io::Result<()> {
    let result = with_cancel(&stream, |cancel| crate::vault::check_site(site, cancel));
    let response = match result {
        Ok(check) => IpcResponse::Vault(VaultIpcReply::Checked {
            site: check.site,
            revision: check.revision,
            total: u64::try_from(check.items.len()).unwrap_or(u64::MAX),
        }),
        Err(error) => error_response(&error),
    };
    write_response(&mut stream, &response)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::vault::{CompareGroup, CompareItem, ItemAction, ItemKind, SideState};

    fn item(index: usize) -> CompareItem {
        let side = |exists: bool| SideState {
            exists,
            size: exists.then_some(1_048_576 + index as u64),
            mtime_ms: exists.then_some(1_760_000_000_000 + index as i64),
            location: "Tresor mit einem längeren Namen".to_string(),
        };
        CompareItem {
            id: format!("{index:016x}"),
            group: CompareGroup::ChangedA,
            kind: ItemKind::File,
            path: format!("/Ordner/Unterordner/Datei mit „Umlauten“ {index:06}.txt"),
            a: side(true),
            b: side(false),
            actions: vec![ItemAction::Transfer],
            finding: None,
            note: None,
        }
    }

    #[test]
    fn vault_task_compare_pages_fit_ipc_line() {
        let items: Vec<CompareItem> = (0..20_000).map(item).collect();
        let (mut offset, mut seen, mut pages) = (0usize, 0usize, 0usize);
        loop {
            let (page, next) = page_of(&items, offset);
            assert!(!page.is_empty(), "a page is never empty");
            let bytes: usize = page
                .iter()
                .map(|entry| serde_json::to_string(entry).unwrap().len() + 1)
                .sum();
            assert!(bytes <= ITEM_PAGE_BYTES, "{bytes} bytes in a page");
            // Gapless, in order.
            for (index, entry) in page.iter().enumerate() {
                assert_eq!(entry.item, items[offset + index].id);
            }
            seen += page.len();
            pages += 1;
            // The reply with its head and escaping fits one line.
            let reply = IpcResponse::Vault(VaultIpcReply::ItemPage {
                items: page,
                next_offset: next,
            });
            let line = serde_json::to_string(&reply).unwrap().len() + 1;
            assert!(line <= MAX_IPC_LINE, "{line} bytes in a line");
            match next {
                Some(more) => {
                    assert_eq!(more as usize, seen);
                    offset = more as usize;
                }
                None => break,
            }
        }
        assert_eq!(seen, items.len());
        assert!(pages > 1, "20 000 entries need several pages");
        // An offset beyond the end is an empty rest, not an error.
        let (rest, next) = page_of(&items, items.len() + 5);
        assert!(rest.is_empty() && next.is_none());
    }

    #[test]
    fn vault_task_compare_text_too_large_refused() {
        // Two versions of 1.2 MiB each do not fit the line of 2 MiB.
        let big = "ä".repeat(600 * 1024);
        let reply = IpcResponse::Vault(VaultIpcReply::Texts {
            a: SecretText::new(big.clone()),
            b: SecretText::new(big),
        });
        match fitting(reply) {
            IpcResponse::Err { msg, code } => {
                assert_eq!(code.as_deref(), Some("compare_too_large"));
                assert_eq!(msg, "Die Datei ist zu groß für die Zusammenführung.");
            }
            other => panic!("not refused: {other:?}"),
        }
        let small = IpcResponse::Vault(VaultIpcReply::Texts {
            a: SecretText::new("a"),
            b: SecretText::new("b"),
        });
        assert!(matches!(fitting(small), IpcResponse::Vault(_)));
    }
}
