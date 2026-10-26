//! Exact backend-relative paths bound to a receipt's already checked job.
use super::{failure, JobRecord, PreviousTarget, Receipt};
use std::io;

pub(super) fn destination_parts(
    record: &JobRecord,
    destination: &str,
) -> io::Result<(String, String)> {
    let display = |path: &str| {
        if record.target.local {
            crate::copy::native_path_text(&crate::transfer::walk_listers::native(path))
        } else {
            Ok(path.to_string())
        }
    };
    let root = display(&record.target_dir)?;
    let path = display(destination)?;
    let relative = path
        .strip_prefix(&format!("{}/", root.trim_end_matches('/')))
        .ok_or_else(|| failure("Dateibeleg liegt außerhalb des Zielroots"))?
        .to_string();
    let destination = crate::transfer::engine_names::join_rel(&record.target_dir, &relative);
    Ok((relative, destination))
}

pub(super) fn validate(receipt: &Receipt, record: &JobRecord) -> io::Result<()> {
    let allow_backslash =
        record.target.local && crate::transfer::platform::backslash_is_name_char();
    if receipt.relative.is_empty()
        || receipt.relative.starts_with('/')
        || receipt.relative.split('/').any(|part| {
            crate::transfer::engine_names::name_problem(part, allow_backslash).is_some()
        })
        || crate::transfer::engine_names::join_rel(&record.target_dir, &receipt.relative)
            != receipt.destination
        || receipt
            .proof
            .as_ref()
            .is_some_and(|proof| proof.path != receipt.destination)
        || receipt.previous.as_ref().is_some_and(|previous| {
            matches!(previous, PreviousTarget::File(proof) if proof.path != receipt.destination)
        })
    {
        return Err(failure("Dateibeleg enthält einen ungültigen Zielpfad"));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::transfer::job_record::{SourceGuard, StoredEndpoint};
    use crate::transfer::{FolderConflict, FolderConflictPolicy, JobItems, Layout};

    fn record() -> JobRecord {
        JobRecord {
            version: 1,
            id: "a".repeat(64),
            source: StoredEndpoint {
                local: false,
                namespace: "source-account".into(),
                state: "source-state".into(),
            },
            target: StoredEndpoint {
                local: false,
                namespace: "target-account".into(),
                state: "target-state".into(),
            },
            target_dir: "/target".into(),
            target_object: None,
            vault_locations: Vec::new(),
            items: JobItems::Roots {
                paths: vec!["/source/a.bin".into()],
                base: None,
            },
            layout: Layout::Tree,
            filter: None,
            file_policy: 0,
            folders: FolderConflictPolicy::new(FolderConflict::KeepBoth),
            moving: false,
            guards: Vec::new(),
            complete: false,
            mutation_started: false,
            file_journal: true,
        }
    }

    fn receipt(record: &JobRecord, relative: &str) -> Receipt {
        let destination = crate::transfer::engine_names::join_rel(&record.target_dir, relative);
        let proof = super::super::Proof {
            path: destination.clone(),
            object: Some("owned-target-object".into()),
            id: None,
            size: 5,
            mtime_ms: 0,
            sha256: "00".repeat(32),
        };
        Receipt {
            version: 1,
            job: record.id.clone(),
            source_endpoint: record.source.clone(),
            target_endpoint: record.target.clone(),
            target_root: record.target_dir.clone(),
            key: "/source/a.bin\0a.bin".into(),
            source: SourceGuard {
                path: "/source/a.bin".into(),
                directory: false,
                link: false,
                id: None,
                object: Some("owned-source-object".into()),
                size: 5,
                mtime_ms: 0,
                verified: true,
            },
            relative: relative.into(),
            destination,
            proof: Some(proof.clone()),
            quarantine: None,
            staged: None,
            confirmed: false,
            acknowledged: false,
            discarded: false,
            publication_authorized: true,
            previous: Some(PreviousTarget::File(proof)),
        }
    }

    #[test]
    fn batches_task_transfer_compat_receipt_paths_preserve_backend_literals() {
        let mut record = record();
        for (root, path, relative) in [
            (
                "/target",
                "/target/Fotos.2026#% 子/a%2Fb#.bin",
                "Fotos.2026#% 子/a%2Fb#.bin",
            ),
            ("/target/", "/target/%2e%2e/a.bin", "%2e%2e/a.bin"),
            ("/", "/Fotos/a.bin", "Fotos/a.bin"),
        ] {
            record.target_dir = root.into();
            assert_eq!(
                destination_parts(&record, path).unwrap(),
                (relative.to_string(), path.to_string())
            );
            let receipt = receipt(&record, relative);
            super::super::receipts::validate(&receipt, &record).unwrap();
        }
        record.target_dir = "/target".into();
        for outside in ["/target-other/a.bin", "/target", "/other/a.bin"] {
            assert!(destination_parts(&record, outside).is_err(), "{outside}");
        }
    }

    #[test]
    fn batches_task_transfer_compat_receipt_paths_reject_traversal_and_unbound_proofs() {
        let record = record();
        for relative in [
            "",
            "/absolute",
            "../escape",
            "a/../escape",
            "a//b",
            "a/./b",
            "a\0b",
            "a\\b",
        ] {
            assert!(
                super::super::receipts::validate(&receipt(&record, relative), &record).is_err(),
                "{relative:?}"
            );
        }
        let original = receipt(&record, "Fotos/a.bin");
        for field in 0..3 {
            let mut changed = original.clone();
            match field {
                0 => changed.destination.push_str(".foreign"),
                1 => changed.proof.as_mut().unwrap().path.push_str(".foreign"),
                _ => {
                    if let Some(PreviousTarget::File(proof)) = &mut changed.previous {
                        proof.path.push_str(".foreign");
                    }
                }
            }
            assert!(super::super::receipts::validate(&changed, &record).is_err());
        }
    }

    #[test]
    fn batches_task_transfer_compat_receipt_paths_bind_endpoints_and_job() {
        let record = record();
        let receipt = receipt(&record, "Fotos/a.bin");
        for field in 0..7 {
            let mut changed = record.clone();
            match field {
                0 => changed.source.namespace.push_str(".foreign"),
                1 => changed.source.state.push_str(".foreign"),
                2 => changed.target.namespace.push_str(".foreign"),
                3 => changed.target.state.push_str(".foreign"),
                4 => changed.target.local = true,
                5 => changed.id.push('b'),
                _ => changed.target_dir.push_str(".foreign"),
            }
            assert!(super::super::receipts::validate(&receipt, &changed).is_err());
        }
    }

    #[test]
    fn batches_task_transfer_compat_receipt_paths_keep_native_backslash_boundary() {
        let mut record = record();
        record.target.local = true;
        let result =
            super::super::receipts::validate(&receipt(&record, "literal\\name/a.bin"), &record);
        #[cfg(windows)]
        assert!(result.is_err());
        #[cfg(not(windows))]
        assert!(result.is_ok());
    }

    #[test]
    fn batches_task_transfer_compat_named_folder_receipt_reuses_literal_destination() {
        use crate::transfer::engine::{
            batches_backend_compatibility_task_tests::Limited,
            folder_reservation::reserve_named_folder,
            test_backend::{fwd, write},
        };
        use crate::transfer::RecoveryMode;
        use std::sync::atomic::{AtomicBool, Ordering};

        let root = tempfile::tempdir().unwrap();
        let source = root.path().join("source");
        let target = root.path().join("target");
        let name = "Fotos.2026#% 子";
        write(&source.join("a.bin"), b"ours");
        write(&target.join(name).join("foreign"), b"foreign");
        let source_backend = crate::vfs::LocalBackend::new("/");
        let mut target_backend = Limited::new("named-folder-receipt");
        target_backend.proofs = true;
        let job = crate::transfer::new_job_id();
        let source_path = fwd(&source);
        let requested = fwd(&target.join(name));
        let expected = fwd(&target.join(format!("{name} (2)")));
        let cancel = AtomicBool::new(false);
        for recovery in [RecoveryMode::New, RecoveryMode::Resume] {
            let (destination, reservation) = reserve_named_folder(
                &source_backend,
                &source_path,
                &target_backend,
                &requested,
                true,
                &job,
                recovery,
                recovery == RecoveryMode::Resume,
                &cancel,
            )
            .unwrap();
            assert_eq!(destination, expected);
            drop(reservation);
        }
        assert_eq!(target_backend.creates.load(Ordering::SeqCst), 1);
        assert_eq!(target_backend.reserved_renames.load(Ordering::SeqCst), 1);
        assert_eq!(std::fs::read_dir(&target).unwrap().count(), 2);
        assert_eq!(std::fs::read(source.join("a.bin")).unwrap(), b"ours");
        assert_eq!(
            std::fs::read(target.join(name).join("foreign")).unwrap(),
            b"foreign"
        );
    }
}
