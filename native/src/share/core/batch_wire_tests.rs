use super::*;
use crate::share::wire::{Ctrl, FsRequest, FsResponse, FsWriteCapabilities};

const NONCE: &str = "0123456789abcdef";

fn encoded_len(ctrl: &Ctrl) -> usize {
    serde_json::to_vec(ctrl).unwrap().len()
}

fn put_header(entries: Vec<FsBatchPut>, lease: Option<String>) -> Ctrl {
    Ctrl::Fs {
        req: FsRequest::PutBatch {
            nonce: "0".repeat(NONCE_HEX_LEN),
            entries,
        },
        lease,
    }
}

fn measured(entries: &[FsBatchPut]) -> Vec<(usize, u64)> {
    entries
        .iter()
        .map(|entry| (serde_json::to_vec(entry).unwrap().len(), entry.size))
        .collect()
}

#[test]
fn transfer_engine_task_batch_frames_round_trip() {
    let requests = vec![
        FsRequest::PutBatch {
            nonce: NONCE.into(),
            entries: vec![
                FsBatchPut {
                    path: "/A/eins.txt".into(),
                    size: 3,
                },
                FsBatchPut {
                    path: "/A/Gr\u{fc}\u{df}e \"x\".txt".into(),
                    size: 0,
                },
            ],
        },
        FsRequest::PutBatchStatus {
            nonce: NONCE.into(),
        },
        FsRequest::GetBatch {
            items: vec![
                FsBatchGet {
                    path: "/A/a".into(),
                    id: None,
                    size: 1,
                },
                FsBatchGet {
                    path: "/A/b".into(),
                    id: Some("drive-id".into()),
                    size: 2,
                },
            ],
        },
        FsRequest::ReadAt {
            path: "/A/big.bin".into(),
            id: None,
            offset: 1 << 40,
        },
        FsRequest::CreateDir {
            path: "/A/neu".into(),
            exclusive: true,
        },
        FsRequest::CreateDir {
            path: "/A/neu".into(),
            exclusive: false,
        },
        FsRequest::PromoteNoReplace {
            staged: "/A/x.se-copy".into(),
            destination: "/A/x".into(),
            copy: true,
        },
        FsRequest::DiscardStage {
            path: "/A/x.se-copy".into(),
        },
    ];
    let expected_ops = [
        "put_batch",
        "put_batch_status",
        "get_batch",
        "read_at",
        "create_dir",
        "create_dir",
        "promote_no_replace",
        "discard_stage",
    ];
    for (req, op) in requests.into_iter().zip(expected_ops) {
        assert!(req.is_transfer_v1());
        let original = Ctrl::Fs {
            req,
            lease: Some("lease".into()),
        };
        let encoded = serde_json::to_vec(&original).unwrap();
        let text = String::from_utf8(encoded.clone()).unwrap();
        assert!(text.contains(&format!("\"op\":\"{op}\"")), "{text}");
        let decoded: Ctrl = serde_json::from_slice(&encoded).unwrap();
        assert_eq!(
            serde_json::to_value(&decoded).unwrap(),
            serde_json::to_value(&original).unwrap()
        );
    }

    let statuses = vec![
        FsBatchStatus::Pending,
        FsBatchStatus::Aborted,
        FsBatchStatus::Done {
            outcomes: vec![
                FsBatchOutcome::Published {
                    path: "/A/eins (2).txt".into(),
                },
                FsBatchOutcome::failed(&io::Error::new(
                    io::ErrorKind::PermissionDenied,
                    "verweigert",
                )),
            ],
        },
    ];
    for status in statuses {
        let original = Ctrl::FsResp {
            resp: FsResponse::Batch {
                status: status.clone(),
            },
        };
        let decoded: Ctrl =
            serde_json::from_slice(&serde_json::to_vec(&original).unwrap()).unwrap();
        match decoded {
            Ctrl::FsResp {
                resp: FsResponse::Batch { status: decoded },
            } => assert_eq!(decoded, status),
            other => panic!("batch status expected, got {other:?}"),
        }
    }
    assert_eq!(
        FsBatchOutcome::failed(&io::Error::new(io::ErrorKind::PermissionDenied, "nein")),
        FsBatchOutcome::Failed {
            kind: Some(FsErrorKind::PermissionDenied),
            msg: "nein".into(),
        }
    );
}

#[test]
fn transfer_engine_task_legacy_host_rejects_batch_requests() {
    #[derive(serde::Deserialize)]
    #[serde(tag = "op", rename_all = "snake_case")]
    enum LegacyRequest {
        ListDir {
            #[allow(dead_code)]
            path: String,
        },
    }
    let json = serde_json::to_string(&FsRequest::GetBatch {
        items: vec![FsBatchGet {
            path: "/A/a".into(),
            id: None,
            size: 1,
        }],
    })
    .unwrap();
    assert!(serde_json::from_str::<LegacyRequest>(&json).is_err());
    let json = serde_json::to_string(&FsRequest::ListDir { path: "/A".into() }).unwrap();
    assert!(serde_json::from_str::<LegacyRequest>(&json).is_ok());
    assert!(!FsRequest::ListDir { path: "/A".into() }.is_transfer_v1());
}

#[test]
fn transfer_engine_task_capabilities_stay_additive() {
    let legacy: FsWriteCapabilities =
        serde_json::from_str(r#"{"create":true,"replace":false,"namespace_replace":true}"#)
            .unwrap();
    assert!(legacy.transfer.is_absent());
    assert!(legacy.create && !legacy.replace && legacy.namespace_replace);

    let current = FsWriteCapabilities {
        create: true,
        replace: true,
        namespace_replace: true,
        transfer: FsTransferCapabilities::host(),
        ..Default::default()
    };
    let encoded = serde_json::to_string(&current).unwrap();
    #[derive(serde::Deserialize)]
    struct LegacyCapabilities {
        create: bool,
        replace: bool,
        namespace_replace: bool,
    }
    let seen_by_legacy: LegacyCapabilities = serde_json::from_str(&encoded).unwrap();
    assert!(seen_by_legacy.create && seen_by_legacy.replace && seen_by_legacy.namespace_replace);
    let decoded: FsWriteCapabilities = serde_json::from_str(&encoded).unwrap();
    assert_eq!(decoded, current);

    let without = serde_json::to_string(&FsWriteCapabilities::default()).unwrap();
    assert!(!without.contains("transfer"), "{without}");
    let host = FsTransferCapabilities::host();
    assert!(host.v1);
    assert_eq!(host.admission, TRANSFER_STREAMS_PER_CONNECTION);
    assert_eq!(host.batch_max_files, BATCH_MAX_FILES);
    assert_eq!(host.batch_max_bytes, BATCH_MAX_BYTES);
}

#[test]
fn transfer_engine_task_batch_split_follows_encoded_header() {
    let lease = Some("L".repeat(43));
    let envelope = encoded_len(&put_header(Vec::new(), lease.clone()));

    // Long names: the encoded header, not the count, closes each request.
    let entries: Vec<FsBatchPut> = (0..200)
        .map(|index| FsBatchPut {
            path: format!("/A/{index:04}-{}", "\u{e4}".repeat(1500)),
            size: 1,
        })
        .collect();
    let parts = plan_batches(envelope, &measured(&entries), 256, BATCH_MAX_BYTES);
    assert!(parts.len() > 1, "{parts:?}");
    let mut next = 0;
    for part in &parts {
        let BatchPart::Items(range) = part else {
            panic!("no oversized entry expected: {part:?}");
        };
        assert_eq!(range.start, next);
        next = range.end;
        let header = encoded_len(&put_header(entries[range.clone()].to_vec(), lease.clone()));
        assert!(
            header <= MAX_BATCH_HEADER_BYTES,
            "{header} bytes in one header"
        );
    }
    assert_eq!(next, entries.len());

    // Short names: the count limit applies.
    let tiny: Vec<FsBatchPut> = (0..600)
        .map(|index| FsBatchPut {
            path: format!("/A/{index}"),
            size: 1,
        })
        .collect();
    assert_eq!(
        plan_batches(envelope, &measured(&tiny), 256, BATCH_MAX_BYTES),
        vec![
            BatchPart::Items(0..256),
            BatchPart::Items(256..512),
            BatchPart::Items(512..600),
        ]
    );

    // Bytes: a batch never exceeds the limit; larger files go alone.
    let mib = 1024 * 1024;
    assert_eq!(
        plan_batches(
            envelope,
            &[(20, 10 * mib), (20, 10 * mib), (20, mib)],
            256,
            16 * mib
        ),
        vec![BatchPart::Items(0..1), BatchPart::Items(1..3)]
    );
    assert_eq!(
        plan_batches(envelope, &[(20, 1), (20, 20 * mib), (20, 1)], 256, 16 * mib),
        vec![
            BatchPart::Items(0..1),
            BatchPart::Oversized(1),
            BatchPart::Items(2..3),
        ]
    );
    assert_eq!(
        plan_batches(
            envelope,
            &[(20, 1), (MAX_BATCH_HEADER_BYTES, 1)],
            256,
            16 * mib
        ),
        vec![BatchPart::Items(0..1), BatchPart::Oversized(1)]
    );
    assert!(plan_batches(envelope, &[], 256, 16 * mib).is_empty());
}

#[test]
fn transfer_engine_task_host_enforces_batch_bounds() {
    let entry = |size| FsBatchPut {
        path: "/A/x".into(),
        size,
    };
    assert!(validate_put(NONCE, &[entry(1)]).is_ok());
    assert!(validate_put(&"f".repeat(64), &[entry(1)]).is_ok());
    for nonce in [
        "0123456789ABCDEF",
        "0123",
        "../../../etc/pas",
        "0123456789abcde/",
    ] {
        let error = validate_put(nonce, &[entry(1)]).unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::InvalidInput, "{nonce}");
    }
    assert!(!valid_nonce(&"a".repeat(65)));
    assert!(validate_put(NONCE, &[]).is_err());
    let too_many = vec![entry(0); BATCH_MAX_FILES as usize + 1];
    assert!(validate_put(NONCE, &too_many).is_err());
    let full = vec![entry(0); BATCH_MAX_FILES as usize];
    assert!(validate_put(NONCE, &full).is_ok());
    assert!(validate_put(NONCE, &[entry(BATCH_MAX_BYTES)]).is_ok());
    assert!(validate_put(NONCE, &[entry(BATCH_MAX_BYTES), entry(1)]).is_err());
    assert!(validate_put(NONCE, &[entry(u64::MAX), entry(u64::MAX)]).is_err());

    let item = |size| FsBatchGet {
        path: "/A/x".into(),
        id: None,
        size,
    };
    assert!(validate_get(&[item(BATCH_MAX_BYTES)]).is_ok());
    assert!(validate_get(&[]).is_err());
    assert!(validate_get(&[item(BATCH_MAX_BYTES), item(1)]).is_err());
}

#[test]
fn transfer_engine_task_batch_status_summary_counts_outcomes() {
    let status = FsBatchStatus::Done {
        outcomes: vec![
            FsBatchOutcome::Published {
                path: "/A/a".into(),
            },
            FsBatchOutcome::Published {
                path: "/A/b".into(),
            },
            FsBatchOutcome::Failed {
                kind: None,
                msg: "x".into(),
            },
        ],
    };
    assert_eq!(
        status.summary(),
        "Paket: 2 veröffentlicht, 1 fehlgeschlagen"
    );
    assert_eq!(FsBatchStatus::Aborted.summary(), "Paket abgebrochen");
}

#[test]
fn transfer_engine_task_only_upload_stages_are_discardable() {
    // The engine's `stage_name` and `vfs::unique_staging_path(…, "upload")`
    // both produce `<file>.se-upload-{:016x}`.
    let generated = format!("file.txt.se-upload-{:016x}", u64::MAX / 3);
    assert!(discardable_stage(&generated));
    assert!(discardable_stage("a.se-upload-0000000000000000"));
    for name in [
        "report.pdf",
        "report.pdf.se-peer-0123456789abcdef",
        "report.pdf.se-batch-0123456789abcdef-3",
        "report.pdf.se-copy-0123456789abcdef",
        "report.pdf.se-upload-0123456789ABCDEF",
        "report.pdf.se-upload-0123456789abcde",
        "report.pdf.se-upload-0123456789abcdef0",
        "report.pdf.se-upload-0123456789abcdeg",
        "report.pdf.se-upload-",
        ".se-upload-0123456789abcdef",
        "x.se-upload-0123456789abcdef.txt",
    ] {
        assert!(!discardable_stage(name), "{name}");
    }
}
