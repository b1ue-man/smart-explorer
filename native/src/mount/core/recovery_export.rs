//! Read-only interpretation of captured journal bytes. Does not open a
//! spool, rotate a journal, clean files or contact a provider.
use super::journal_record::{
    apply, decode_record, DeletePhase, RecoveredJournal, MAX_RECORD_BYTES,
};
use super::recovery_manifest::{RecoveryGap, RecoveryGapKind, RecoveryJournal, RecoveryManifest};
use std::collections::HashSet;

/// Older sealed manifests may contain snapshots made by applying randomized
/// ciphertext path keys. Normalize only the decoded view; their original
/// manifest and journal bytes, hashes and ownership evidence remain intact.
pub(crate) fn sealed_inventory_view(manifest: &mut RecoveryManifest) {
    if !manifest.sealed {
        return;
    }
    for journal in &mut manifest.journals {
        journal.entries.clear();
        journal.deletes.clear();
        journal.namespace_intents.clear();
    }
    manifest.gaps.retain(|gap| {
        !matches!(
            gap.kind,
            RecoveryGapKind::MissingPayload
                | RecoveryGapKind::RemoteQuarantineNotBackedUp
                | RecoveryGapKind::RemoteStageNotBackedUp
        )
    });
    let subjects: HashSet<_> = manifest
        .files
        .iter()
        .filter(|file| {
            file.components
                .last()
                .is_some_and(|name| name == "stages.json" || name.starts_with("journal.jsonl"))
        })
        .map(|file| file.components.join("/"))
        .chain(
            manifest
                .journals
                .iter()
                .map(|journal| journal.components.join("/")),
        )
        .collect();
    let mut subjects: Vec<_> = subjects.into_iter().collect();
    subjects.sort();
    for subject in subjects {
        if !manifest.gaps.iter().any(|gap| {
            gap.kind == RecoveryGapKind::JournalInventoryUnknown && gap.subject == subject
        }) {
            gap(manifest, RecoveryGapKind::JournalInventoryUnknown, subject,
                "Versiegelte Zustandsdatei als Rohbytes gesichert; offene Änderungen, Löschungen und Namespaceabsichten bleiben ohne Schlüssel unbekannt. Eine ältere Ciphertextinventur ist kein Zustandsbeleg");
        }
    }
}

pub(crate) fn inspect_journal(
    manifest: &mut RecoveryManifest,
    components: Vec<String>,
    bytes: &[u8],
) -> std::io::Result<()> {
    let subject = components.join("/");
    if manifest.sealed {
        // Each sealed occurrence has a fresh random header. Applying these
        // opaque strings as path keys would invent entries after Forget/Move.
        // Only the key holder may reconstruct semantic state; all original
        // bytes have already been captured and hashed independently.
        gap(manifest, RecoveryGapKind::JournalInventoryUnknown, subject,
            "Versiegeltes Journal vollständig als Rohbytes gesichert; ohne Schlüssel sind offene Änderungen, Löschungen und Namespaceabsichten unbekannt. Keine Payloadlücke aus Ciphertextpfaden abgeleitet");
        manifest.journals.push(RecoveryJournal {
            components,
            entries: Vec::new(),
            deletes: Vec::new(),
            namespace_intents: Vec::new(),
        });
        return Ok(());
    }
    let mut recovered = RecoveredJournal::default();
    let mut remaining = bytes;
    while !remaining.is_empty() {
        let Some(end) = remaining.iter().position(|byte| *byte == b'\n') else {
            gap(
                manifest,
                RecoveryGapKind::TornJournalTail,
                subject.clone(),
                "Unvollständiger Journalabschluss bleibt im Originaljournal erhalten",
            );
            break;
        };
        let line = &remaining[..end];
        remaining = &remaining[end + 1..];
        if line.is_empty() {
            continue;
        }
        if line.len() > MAX_RECORD_BYTES {
            gap(
                manifest,
                RecoveryGapKind::UnreadableJournal,
                subject.clone(),
                "Journalzeile überschreitet die unterstützte Größe; Originalbytes bleiben erhalten",
            );
            break;
        }
        match decode_record(line) {
            Ok(action) => apply(&mut recovered, action),
            Err(_) => {
                gap(manifest, RecoveryGapKind::UnreadableJournal, subject.clone(),
                    "Journalzustand ist nicht vollständig interpretierbar; Originalbytes bleiben erhalten");
                break;
            }
        }
    }
    let parent = &components[..components.len() - 1];
    let saved: HashSet<_> = manifest
        .files
        .iter()
        .map(|file| file.components.clone())
        .collect();
    for entry in recovered.entries.values() {
        let mut payload = parent.to_vec();
        payload.extend(["files".to_string(), entry.spool_name.clone()]);
        if !saved.contains(&payload) {
            gap(
                manifest,
                RecoveryGapKind::MissingPayload,
                payload.join("/"),
                "Referenzierte lokale Änderung fehlt; keine Remoteversion ersetzt diese Änderung",
            );
        }
    }
    for delete in recovered.deletes.values() {
        if delete.phase != DeletePhase::LocalOnly {
            gap(manifest, RecoveryGapKind::RemoteQuarantineNotBackedUp,
                delete.quarantine_path.clone(),
                "Remote-Löschquarantäne ist nur referenziert; deren Inhalte wurden nicht lokal gesichert");
        }
    }
    let mut snapshot = RecoveryJournal {
        components,
        entries: recovered
            .entries
            .values()
            .map(serde_json::to_value)
            .collect::<Result<_, _>>()
            .map_err(std::io::Error::other)?,
        deletes: recovered
            .deletes
            .values()
            .map(serde_json::to_value)
            .collect::<Result<_, _>>()
            .map_err(std::io::Error::other)?,
        namespace_intents: recovered
            .namespace_conflicts
            .values()
            .map(serde_json::to_value)
            .collect::<Result<_, _>>()
            .map_err(std::io::Error::other)?,
    };
    snapshot.entries.sort_by_key(serde_json::Value::to_string);
    snapshot.deletes.sort_by_key(serde_json::Value::to_string);
    snapshot
        .namespace_intents
        .sort_by_key(serde_json::Value::to_string);
    manifest.journals.push(snapshot);
    Ok(())
}

pub(crate) fn inspect_stages(manifest: &mut RecoveryManifest, subject: &str, bytes: &[u8]) {
    if manifest.sealed {
        gap(manifest, RecoveryGapKind::JournalInventoryUnknown, subject.into(),
            "Versiegelter Stage-Ledger vollständig als Rohbytes gesichert; ohne Schlüssel bleibt die semantische Inventur unbekannt");
        return;
    }
    match serde_json::from_slice::<serde_json::Value>(bytes) {
        Ok(value) => {
            if let Some(stages) = value.get("stages").and_then(serde_json::Value::as_object) {
                for stage in stages.keys() {
                    gap(manifest, RecoveryGapKind::RemoteStageNotBackedUp, stage.clone(),
                        "Remote-Uploadstage ist referenziert; lokale Payloads sind separat im Hashmanifest erfasst");
                }
            } else {
                gap(
                    manifest,
                    RecoveryGapKind::UnreadableJournal,
                    subject.into(),
                    "Stage-Ledger hat keine lesbare Stagezuordnung; Originalbytes bleiben erhalten",
                );
            }
        }
        Err(_) => gap(
            manifest,
            RecoveryGapKind::UnreadableJournal,
            subject.into(),
            "Stage-Ledger ist nicht lesbar; Originalbytes bleiben erhalten",
        ),
    }
}

pub(crate) fn gap(
    manifest: &mut RecoveryManifest,
    kind: RecoveryGapKind,
    subject: String,
    detail: &str,
) {
    manifest.gaps.push(RecoveryGap {
        kind,
        subject,
        detail: detail.into(),
    });
}
