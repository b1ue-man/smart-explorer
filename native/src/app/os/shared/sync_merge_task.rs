//! Bounded original reads and the recorded apply boundary; no direct writes.
use super::sync_merge_types::{MergeDecision, MergeSession, MergeUi, merge_text};
use std::{io::{self, Read}, sync::atomic::{AtomicBool, Ordering}};
use crate::bisync::{Conflict, MergeChoice, OriginalContent, StateKey};

// The existing linemerge contract rejects >16 MiB per side. Bound reads first.
const INPUT_BYTES: usize = 16 * 1024 * 1024;
fn original(backend: &dyn crate::vfs::Backend, path: &str,
    signature: Option<crate::bisync::Sig>, cancel: &AtomicBool) -> Result<Option<Vec<u8>>, String> {
    let Some(signature) = signature else { return Ok(None); };
    if signature.size > INPUT_BYTES as u64 { return Err("Datei überschreitet die 16-MiB-Grenze des Zeilenvergleichs.".into()); }
    let mut reader = crate::vfs::open_read_regular(backend, path, None).map_err(|e| e.to_string())?;
    let mut bytes = Vec::new(); let mut buffer = [0u8; 32 * 1024];
    loop {
        if cancel.load(Ordering::Acquire) { return Err("Laden abgebrochen".into()); }
        let remaining = INPUT_BYTES.saturating_sub(bytes.len()).saturating_add(1).min(buffer.len());
        let count = reader.read(&mut buffer[..remaining]).map_err(|e| e.to_string())?;
        if count == 0 { break; }
        if count > INPUT_BYTES.saturating_sub(bytes.len()) { return Err("Datei ist für den Zeilenvergleich zu groß.".into()); }
        bytes.extend_from_slice(&buffer[..count]);
    }
    Ok(Some(bytes))
}

pub(in crate::app) fn load(mut session: MergeSession, cancel: &AtomicBool) -> Result<MergeUi, String> {
    let pending = crate::bisync::pending_merge_for_key(&*session.a, &session.root_a,
        &*session.b, &session.root_b, &session.key, &session.conflict.rel).map_err(|e| e.to_string())?;
    let mut ui = MergeUi::loading(session.conflict.rel.clone());
    if let Some(pending) = pending {
        // A restart retries the durable original session, never partially published bytes.
        session.conflict = pending.conflict; session.original_a = pending.original_a;
        session.original_b = pending.original_b;
        ui.retry = Some(match pending.choice {
            crate::bisync::RecordedMergeChoice::Write => MergeDecision::Write(pending.merged),
            crate::bisync::RecordedMergeChoice::KeepBoth { keep_a } => MergeDecision::KeepBoth { keep_a },
        });
        ui.last_error = Some(format!("Unterbrochene Zusammenführung: Quelle {}, Ziel {}. Die ursprüngliche Entscheidung bleibt erhalten.",
            if pending.confirmed_a.is_some() { "bereits bestätigt" } else { "noch offen" },
            if pending.confirmed_b.is_some() { "bereits bestätigt" } else { "noch offen" }));
    } else {
        let paths = crate::bisync::recorded_original_paths_for_key(&*session.a, &session.root_a,
            &*session.b, &session.root_b, &session.key, &session.conflict.rel).map_err(|e| e.to_string())?;
        session.original_a = original(&*session.a, &paths.path_a, session.conflict.a, cancel)?;
        session.original_b = original(&*session.b, &paths.path_b, session.conflict.b, cancel)?;
    }
    if cancel.load(Ordering::Acquire) { return Err("Laden abgebrochen".into()); }
    // Pending input may use a larger engine budget; it remains retryable without a diff.
    let texts = if session.original_a.as_ref().is_some_and(|b| b.len() > INPUT_BYTES)
        || session.original_b.as_ref().is_some_and(|b| b.len() > INPUT_BYTES) {
        Err("Gespeicherte Eingabe überschreitet die Zeilenvergleichsgrenze; ursprüngliche Entscheidung wiederholen.".into())
    } else {
        merge_text(session.original_a.as_deref()).and_then(|a| merge_text(session.original_b.as_deref()).map(|b| (a,b)))
    };
    match texts {
        Ok((a,b)) => {
            ui.shape_a = crate::linemerge::TextShape::of(a); ui.shape_b = crate::linemerge::TextShape::of(b);
            ui.shape = ui.shape_a;
            match crate::linemerge::rows(a,b) { Ok(rows) => ui.rows = rows, Err(error) => ui.text_error = Some(error.to_string()) }
        }
        Err(error) => ui.text_error = Some(error),
    }
    ui.session = Some(session); Ok(ui)
}

pub(in crate::app) fn apply(ui: &mut MergeUi, cancel: &AtomicBool) -> Result<crate::bisync::MergeReport, crate::bisync::MergeFailure> {
    let missing = || crate::bisync::MergeFailure { error:io::Error::new(io::ErrorKind::InvalidInput,
        "Originalsitzung oder Entscheidung fehlt"), partial:Default::default() };
    if cancel.load(Ordering::Acquire) { return Err(crate::bisync::MergeFailure {
        error:io::Error::new(io::ErrorKind::Interrupted,"Zusammenführen abgebrochen"), partial:Default::default() }); }
    if let Some(MergeDecision::Rows(shape)) = &ui.retry {
        let bytes = super::sync_merge_types::assemble_text(&ui.rows,*shape);
        ui.retry = Some(MergeDecision::Write(bytes));
    }
    ui.rows = Vec::new();
    let session = ui.session.as_ref().ok_or_else(missing)?;
    let choice = match ui.retry.as_ref().ok_or_else(missing)? {
        MergeDecision::Write(bytes) => MergeChoice::Write(bytes),
        MergeDecision::KeepBoth { keep_a } => MergeChoice::KeepBoth { keep_a:*keep_a },
        MergeDecision::Rows(_) => return Err(missing()),
    };
    crate::bisync::merge_recorded_for_key(&*session.a, &session.root_a, &*session.b, &session.root_b,
        &session.key, &session.conflict,
        OriginalContent { signature:session.conflict.a, bytes:session.original_a.as_deref() },
        OriginalContent { signature:session.conflict.b, bytes:session.original_b.as_deref() }, choice, cancel, |_| {})
}

pub(in crate::app) fn session(context: &super::BisyncCtx, conflict: Conflict, key: StateKey) -> MergeSession {
    MergeSession { a:context.a.clone(), root_a:context.root_a.clone(), b:context.b.clone(), root_b:context.root_b.clone(),
        key, conflict, original_a:None, original_b:None }
}
