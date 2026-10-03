//! Conversions between the app's extension types (`vfs` extensions, the
//! duplicate search of `analytics`) and the plain payloads of the agent
//! protocol's extension frames (`agent_proto::ops_types`). Used by the
//! client (`AgentBackend`) and by the background service that serves a
//! backend over the same protocol.
use std::io;
use std::sync::atomic::Ordering;

use crate::agent_proto::{
    digest, omission, WireChange, WireDuplicateGroup, WireDuplicateItem, WireDuplicateSummary,
    WireOmission, WireReclaimProgress, WireTargetLimits, UNSUPPORTED_EXTENSION,
};
use crate::analytics::{
    ContentHash, DuplicateEvidence, DuplicateGroup, DuplicateReport, DuplicateSummary,
    HashAlgorithm, ProtectedOmission, ReclaimItem, ReclaimPhase, ReclaimProgress,
};
use crate::vfs::{
    ChangeNotice, MtimePrecision, NameLimit, OmissionReason, StageDurability, TargetLimits,
    VfsOmission,
};

#[cfg(test)]
mod change_tests {
    use super::*;
    use crate::agent_proto::Frame;

    #[test]
    fn review_task_agent_watch_distinguishes_partial_coverage() {
        for notice in [ChangeNotice::Ready { generation: Some(7) },
            ChangeNotice::ReadyPartial { generation: Some(7) }] {
            let wire = change_to_wire(notice.clone());
            let encoded = Frame::Change(wire).encode(31).unwrap();
            let (_, Frame::Change(decoded)) = Frame::decode(&encoded).unwrap() else {
                panic!("change notice expected");
            };
            assert_eq!(change_from_wire(decoded), notice);
        }
    }
}

/// An `Unsupported` error that says the serving side lacks the extension.
pub(crate) fn unsupported() -> io::Error {
    io::Error::new(io::ErrorKind::Unsupported, UNSUPPORTED_EXTENSION)
}

/// Whether a reply text is the serving side's "unsupported extension".
pub(crate) fn is_unsupported(message: &str) -> bool {
    message == UNSUPPORTED_EXTENSION
}

pub(crate) fn omission_to_wire(omitted: VfsOmission) -> WireOmission {
    WireOmission {
        rel: omitted.rel,
        reason: match omitted.reason {
            OmissionReason::Link => omission::LINK,
            OmissionReason::Special => omission::SPECIAL,
            OmissionReason::Unreadable => omission::UNREADABLE,
            OmissionReason::Vanished => omission::VANISHED,
            OmissionReason::Unrepresentable => omission::UNREPRESENTABLE,
        },
        detail: omitted.detail,
    }
}

pub(crate) fn omission_from_wire(omitted: WireOmission) -> VfsOmission {
    VfsOmission {
        rel: omitted.rel,
        reason: match omitted.reason {
            omission::LINK => OmissionReason::Link,
            omission::SPECIAL => OmissionReason::Special,
            omission::VANISHED => OmissionReason::Vanished,
            omission::UNREPRESENTABLE => OmissionReason::Unrepresentable,
            _ => OmissionReason::Unreadable,
        },
        detail: omitted.detail,
    }
}

pub(crate) fn algorithm_to_wire(algorithm: Option<HashAlgorithm>) -> u8 {
    match algorithm {
        None => digest::NONE,
        Some(HashAlgorithm::Md5) => digest::MD5,
        Some(HashAlgorithm::Sha256) => digest::SHA256,
    }
}

pub(crate) fn algorithm_from_wire(code: u8) -> Option<HashAlgorithm> {
    match code {
        digest::MD5 => Some(HashAlgorithm::Md5),
        digest::SHA256 => Some(HashAlgorithm::Sha256),
        _ => None,
    }
}

pub(crate) fn durability_to_wire(durability: StageDurability) -> u8 {
    match durability {
        StageDurability::NotRequired => 0,
        StageDurability::Deferred => 1,
        StageDurability::Now => 2,
    }
}

pub(crate) fn durability_from_wire(code: u8) -> StageDurability {
    match code {
        1 => StageDurability::Deferred,
        2 => StageDurability::Now,
        _ => StageDurability::NotRequired,
    }
}

const PRECISIONS: [MtimePrecision; 8] = [
    MtimePrecision::Nanos,
    MtimePrecision::Millis,
    MtimePrecision::TenMillis,
    MtimePrecision::Seconds,
    MtimePrecision::TwoSeconds,
    MtimePrecision::Minutes,
    MtimePrecision::Days,
    MtimePrecision::Unknown,
];

pub(crate) fn limits_to_wire(limits: TargetLimits) -> WireTargetLimits {
    let (name_limit, name_max) = match limits.max_name {
        None => (0, 0),
        Some(NameLimit::Bytes(max)) => (1, max as u64),
        Some(NameLimit::Utf16Units(max)) => (2, max as u64),
    };
    WireTargetLimits {
        windows_names: limits.windows_names,
        name_limit,
        name_max,
        max_file_size: limits.max_file_size,
        precision: PRECISIONS
            .iter()
            .position(|known| *known == limits.mtime_precision)
            .unwrap_or(PRECISIONS.len() - 1) as u8,
    }
}

pub(crate) fn limits_from_wire(limits: WireTargetLimits) -> TargetLimits {
    let max = usize::try_from(limits.name_max).unwrap_or(usize::MAX);
    TargetLimits {
        windows_names: limits.windows_names,
        max_name: match limits.name_limit {
            1 => Some(NameLimit::Bytes(max)),
            2 => Some(NameLimit::Utf16Units(max)),
            _ => None,
        },
        max_file_size: limits.max_file_size,
        mtime_precision: PRECISIONS
            .get(usize::from(limits.precision))
            .copied()
            .unwrap_or_default(),
    }
}

pub(crate) fn change_to_wire(notice: ChangeNotice) -> WireChange {
    match notice {
        ChangeNotice::Ready { generation } => WireChange {
            kind: 0,
            generation,
            ..WireChange::default()
        },
        ChangeNotice::ReadyPartial { generation } => WireChange {
            kind: 4,
            generation,
            ..WireChange::default()
        },
        ChangeNotice::Changed { generation, paths } => WireChange {
            kind: 1,
            generation,
            paths,
            text: String::new(),
        },
        ChangeNotice::Overflow => WireChange {
            kind: 2,
            ..WireChange::default()
        },
        ChangeNotice::Ended(text) => WireChange {
            kind: 3,
            text,
            ..WireChange::default()
        },
    }
}

pub(crate) fn change_from_wire(change: WireChange) -> ChangeNotice {
    match change.kind {
        0 => ChangeNotice::Ready {
            generation: change.generation,
        },
        1 => ChangeNotice::Changed {
            generation: change.generation,
            paths: change.paths,
        },
        2 => ChangeNotice::Overflow,
        4 => ChangeNotice::ReadyPartial { generation: change.generation },
        _ => ChangeNotice::Ended(change.text),
    }
}

const PHASES: [ReclaimPhase; 4] = [
    ReclaimPhase::Walking,
    ReclaimPhase::Fingerprinting,
    ReclaimPhase::Hashing,
    ReclaimPhase::Grouping,
];

/// The counters and phase of a running duplicate search.
pub(crate) fn progress_to_wire(progress: &ReclaimProgress) -> WireReclaimProgress {
    let load = |counter: &std::sync::atomic::AtomicU64| counter.load(Ordering::Relaxed);
    let phase = progress.stage.phase();
    WireReclaimProgress {
        files: load(&progress.files),
        dirs: load(&progress.dirs),
        bytes: load(&progress.bytes),
        fingerprinted: load(&progress.fingerprinted),
        hashed: load(&progress.hashed),
        candidates: load(&progress.candidates),
        phase: PHASES.iter().position(|known| *known == phase).unwrap_or(0) as u8,
        ..WireReclaimProgress::default()
    }
}

/// Takes the host's counters into the caller's progress.
pub(crate) fn apply_progress(progress: &ReclaimProgress, state: &WireReclaimProgress) {
    progress.files.store(state.files, Ordering::Relaxed);
    progress.dirs.store(state.dirs, Ordering::Relaxed);
    progress.bytes.store(state.bytes, Ordering::Relaxed);
    progress
        .fingerprinted
        .store(state.fingerprinted, Ordering::Relaxed);
    progress.hashed.store(state.hashed, Ordering::Relaxed);
    progress
        .candidates
        .store(state.candidates, Ordering::Relaxed);
}

pub(crate) fn group_to_wire(group: DuplicateGroup) -> WireDuplicateGroup {
    WireDuplicateGroup {
        size: group.size,
        algorithm: algorithm_to_wire(Some(group.hash.algorithm)),
        hex: group.hash.hex,
        evidence: match group.evidence {
            DuplicateEvidence::LocalSha256 => 0,
            DuplicateEvidence::ProviderMd5 => 1,
            DuplicateEvidence::AgentMd5 => 2,
        },
        reclaimable: group.reclaimable,
        items: group
            .items
            .into_iter()
            .map(|item| WireDuplicateItem {
                path: item.path,
                name: item.name,
                size: item.size,
                mtime_ms: item.mtime_ms,
                backend_id: item.backend_id,
            })
            .collect(),
    }
}

pub(crate) fn group_from_wire(group: WireDuplicateGroup) -> DuplicateGroup {
    DuplicateGroup {
        hash: ContentHash {
            algorithm: algorithm_from_wire(group.algorithm).unwrap_or(HashAlgorithm::Sha256),
            hex: group.hex,
        },
        evidence: match group.evidence {
            1 => DuplicateEvidence::ProviderMd5,
            2 => DuplicateEvidence::AgentMd5,
            _ => DuplicateEvidence::LocalSha256,
        },
        size: group.size,
        reclaimable: group.reclaimable,
        items: group
            .items
            .into_iter()
            .map(|item| {
                let mut copy =
                    ReclaimItem::new(item.path, item.name, item.size, item.mtime_ms, false)
                        .with_reason("Duplikat", crate::analytics::ReclaimConfidence::HashMatch);
                copy.backend_id = item.backend_id;
                copy
            })
            .collect(),
    }
}

pub(crate) fn summary_to_wire(
    summary: DuplicateSummary,
    root_error: Option<String>,
) -> WireDuplicateSummary {
    WireDuplicateSummary {
        files: summary.files,
        bytes: summary.bytes,
        candidates: summary.candidates,
        compared: summary.compared,
        groups: summary.groups,
        protected: summary
            .protected
            .into_iter()
            .map(|area| (area.area, area.entries))
            .collect(),
        errors: summary.errors,
        suppressed_errors: summary.suppressed_errors,
        limits: summary.limits,
        root_error,
    }
}

/// The report of a host-side search from its groups and summary.
pub(crate) fn report_from_wire(
    groups: Vec<DuplicateGroup>,
    summary: WireDuplicateSummary,
) -> DuplicateReport {
    DuplicateReport {
        groups,
        summary: DuplicateSummary {
            files: summary.files,
            bytes: summary.bytes,
            candidates: summary.candidates,
            compared: summary.compared,
            groups: summary.groups,
            protected: summary
                .protected
                .into_iter()
                .map(|(area, entries)| ProtectedOmission { area, entries })
                .collect(),
            errors: summary.errors,
            suppressed_errors: summary.suppressed_errors,
            limits: summary.limits,
        },
        root_error: summary.root_error,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn review_task_extension_payloads_roundtrip() {
        for precision in PRECISIONS {
            let limits = TargetLimits {
                windows_names: true,
                max_name: Some(NameLimit::Utf16Units(255)),
                max_file_size: Some(4 * 1024 * 1024 * 1024 - 1),
                mtime_precision: precision,
            };
            assert_eq!(limits_from_wire(limits_to_wire(limits)), limits);
        }
        for reason in [
            OmissionReason::Link,
            OmissionReason::Special,
            OmissionReason::Unreadable,
            OmissionReason::Vanished,
            OmissionReason::Unrepresentable,
        ] {
            let omitted = VfsOmission {
                rel: "a/b".into(),
                reason,
                detail: "x".into(),
            };
            assert_eq!(
                omission_from_wire(omission_to_wire(omitted.clone())),
                omitted
            );
        }
        for notice in [
            ChangeNotice::Ready {
                generation: Some(3),
            },
            ChangeNotice::Changed {
                generation: None,
                paths: vec!["a".into()],
            },
            ChangeNotice::Overflow,
            ChangeNotice::Ended("weg".into()),
        ] {
            assert_eq!(change_from_wire(change_to_wire(notice.clone())), notice);
        }
        let group = DuplicateGroup {
            hash: ContentHash {
                algorithm: HashAlgorithm::Sha256,
                hex: "ab".into(),
            },
            evidence: DuplicateEvidence::LocalSha256,
            size: 5,
            reclaimable: 5,
            items: vec![ReclaimItem::new("/a".into(), "a".into(), 5, 1, false)
                .with_reason("Duplikat", crate::analytics::ReclaimConfidence::HashMatch)],
        };
        assert_eq!(group_from_wire(group_to_wire(group.clone())), group);
    }
}
