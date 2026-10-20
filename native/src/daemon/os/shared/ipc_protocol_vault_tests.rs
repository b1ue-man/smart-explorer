//! Wire form of `IpcRequest::Vault` and of `IpcResponse::Err.code` (stage
//! `vault_unit`).
use serde::Deserialize;

use super::super::{IpcRequest, IpcResponse};
use super::{
    CompareHead, CompareItemWire, CompareSideWire, ResolveFailure, SealPurposeWire, SecretText,
    VaultIpcReply, VaultIpcRequest, VaultStatusView,
};

const SITE: &str = "0123456789abcdef0123456789abcdef";

fn every_request() -> Vec<VaultIpcRequest> {
    vec![
        VaultIpcRequest::Status,
        VaultIpcRequest::Unlock {
            site: SITE.into(),
            password: SecretText::new("geheimes-Passwort"),
            remember: Some(true),
        },
        VaultIpcRequest::UnlockSaved { site: SITE.into() },
        VaultIpcRequest::Lock {
            site: None,
            keep_unsaved: true,
        },
        VaultIpcRequest::Open { site: SITE.into() },
        VaultIpcRequest::Seal {
            site: SITE.into(),
            purpose: SealPurposeWire::LogFields,
            items: vec![SecretText::new("/Akten/Steuer.pdf")],
        },
        VaultIpcRequest::Unseal {
            site: SITE.into(),
            purpose: SealPurposeWire::LocalStore,
            items: vec!["AAAA".into()],
        },
        VaultIpcRequest::Compare {
            job: "18a2f0c".into(),
            groups: vec!["conflict".into(), "structure".into()],
        },
        VaultIpcRequest::Items {
            list: "18a2f0c".into(),
            revision: 3,
            offset: 500,
        },
        VaultIpcRequest::CompareText {
            job: "18a2f0c".into(),
            item: "0123456789abcdef".into(),
        },
        VaultIpcRequest::Resolve {
            job: "18a2f0c".into(),
            item: Some("0123456789abcdef".into()),
            action: "keep-a".into(),
            all: false,
            confirmed: true,
        },
        VaultIpcRequest::MergeApply {
            job: "18a2f0c".into(),
            item: "0123456789abcdef".into(),
            content: SecretText::new("Zeile aus dem Tresor"),
        },
        VaultIpcRequest::Check { site: SITE.into() },
    ]
}

fn side(exists: bool) -> CompareSideWire {
    CompareSideWire {
        exists,
        size: exists.then_some(12),
        mtime_ms: exists.then_some(1_760_000_000_000),
        location: "Tresor A".into(),
    }
}

fn item() -> CompareItemWire {
    CompareItemWire {
        item: "0123456789abcdef".into(),
        group: "conflict".into(),
        kind: "file".into(),
        path: "/Ordner/Bericht.txt".into(),
        a: side(true),
        b: side(false),
        actions: vec!["keep-a".into(), "keep-b".into()],
        finding: None,
        note: None,
    }
}

#[test]
fn vault_task_ipc_err_code_backward_compatible() {
    // The reply of a worker before this batch has no `code`.
    let old: IpcResponse =
        serde_json::from_str(r#"{"t":"err","msg":"Background-Worker gesperrt"}"#).unwrap();
    assert!(
        matches!(&old, IpcResponse::Err { msg, code: None } if msg == "Background-Worker gesperrt"),
        "{old:?}"
    );
    // Without a code the wire form stays byte for byte the old one.
    let plain = IpcResponse::Err {
        msg: "x".into(),
        code: None,
    };
    assert_eq!(
        serde_json::to_string(&plain).unwrap(),
        r#"{"t":"err","msg":"x"}"#
    );
    // With a code an older reader takes the message and ignores the field.
    let coded = IpcResponse::Err {
        msg: format!("Tresor {SITE} ist gesperrt"),
        code: Some("vault_locked".into()),
    };
    let text = serde_json::to_string(&coded).unwrap();
    assert_eq!(
        text,
        format!(r#"{{"t":"err","msg":"Tresor {SITE} ist gesperrt","code":"vault_locked"}}"#)
    );
    #[derive(Deserialize)]
    #[serde(tag = "t", rename_all = "snake_case")]
    enum OldResponse {
        Err { msg: String },
    }
    let OldResponse::Err { msg } = serde_json::from_str::<OldResponse>(&text).unwrap();
    assert_eq!(msg, format!("Tresor {SITE} ist gesperrt"));
    // A reader of this version keeps the code.
    let back: IpcResponse = serde_json::from_str(&text).unwrap();
    assert!(
        matches!(&back, IpcResponse::Err { code: Some(code), .. } if code == "vault_locked"),
        "{back:?}"
    );
    // The vault replies travel beside the old answers.
    let status = IpcResponse::Vault(VaultIpcReply::Status(VaultStatusView::default()));
    let wire = serde_json::to_value(&status).unwrap();
    assert_eq!(
        (wire["t"].as_str(), wire["kind"].as_str()),
        (Some("vault"), Some("status"))
    );
}

#[test]
fn vault_task_ipc_vault_requests_token_required() {
    for request in every_request() {
        let wire = serde_json::to_value(IpcRequest::Vault {
            token: "daemon-token".into(),
            req: request.clone(),
        })
        .unwrap();
        assert_eq!(wire["t"], "vault");
        assert_eq!(wire["token"], "daemon-token");
        // `ipc.rs::require_request_auth` checks exactly this token before
        // `ipc_vault.rs` sees the request.
        let parsed: IpcRequest = serde_json::from_value(wire.clone()).unwrap();
        assert_eq!(parsed.daemon_token(), Some("daemon-token"));
        let IpcRequest::Vault { req, .. } = parsed else {
            panic!("not a vault request");
        };
        assert_eq!(req, request);
        // Without a token the request does not even parse.
        let mut tokenless = wire;
        tokenless.as_object_mut().unwrap().remove("token");
        assert!(serde_json::from_value::<IpcRequest>(tokenless).is_err());
    }
    // Passwords and cleartext items never appear in debug output.
    let printed = format!("{:?}", every_request());
    assert!(!printed.contains("geheimes-Passwort") && !printed.contains("Steuer"));
    assert!(!printed.contains("Zeile aus dem Tresor"));
    // Only the status of se Desktop leaves a drive-restore worker restricted.
    for request in every_request() {
        assert_eq!(
            request.promotes_daemon(),
            request != VaultIpcRequest::Status
        );
    }
}

#[test]
fn vault_task_ipc_compare_wire_forms() {
    // An older caller's request without `groups` parses, and the ops have the
    // names of the plan.
    let bare: VaultIpcRequest = serde_json::from_str(r#"{"op":"compare","job":"j"}"#).unwrap();
    assert!(matches!(
        bare,
        VaultIpcRequest::Compare { ref groups, .. } if groups.is_empty()
    ));
    let resolve: VaultIpcRequest =
        serde_json::from_str(r#"{"op":"resolve","job":"j","action":"keep-a","all":true}"#).unwrap();
    assert!(matches!(
        resolve,
        VaultIpcRequest::Resolve {
            item: None,
            all: true,
            confirmed: false,
            ..
        }
    ));
    for (request, op) in [
        (
            VaultIpcRequest::Compare {
                job: "j".into(),
                groups: Vec::new(),
            },
            "compare",
        ),
        (
            VaultIpcRequest::Items {
                list: "check:s".into(),
                revision: 1,
                offset: 0,
            },
            "items",
        ),
        (
            VaultIpcRequest::CompareText {
                job: "j".into(),
                item: "i".into(),
            },
            "compare_text",
        ),
        (
            VaultIpcRequest::MergeApply {
                job: "j".into(),
                item: "i".into(),
                content: SecretText::new("x"),
            },
            "merge_apply",
        ),
        (VaultIpcRequest::Check { site: SITE.into() }, "check"),
    ] {
        assert_eq!(serde_json::to_value(&request).unwrap()["op"], op);
        // All six ask for work: they lift the restriction of a drive-restore worker.
        assert!(request.promotes_daemon());
    }
}

#[test]
fn vault_task_ipc_compare_replies_round_trip() {
    let mut counts = std::collections::BTreeMap::new();
    counts.insert("conflict".to_string(), 1usize);
    let replies = vec![
        VaultIpcReply::Compared(CompareHead {
            job: "18a2f0c".into(),
            site_a: SITE.into(),
            label_a: "Tresor A".into(),
            site_b: "fedcba9876543210fedcba9876543210".into(),
            label_b: "Tresor B".into(),
            revision: 3,
            counts,
            total: 1,
            unavailable: None,
        }),
        VaultIpcReply::ItemPage {
            items: vec![item()],
            next_offset: Some(1),
        },
        VaultIpcReply::Texts {
            a: SecretText::new("Fassung A"),
            b: SecretText::new("Fassung B"),
        },
        VaultIpcReply::Resolved {
            item: None,
            result: "ok".into(),
            resolved: 2,
            failed: vec![ResolveFailure {
                item: "ab".into(),
                error: "Datei geändert".into(),
            }],
        },
        VaultIpcReply::Checked {
            site: SITE.into(),
            revision: 4,
            total: 7,
        },
    ];
    for reply in replies {
        let text = serde_json::to_string(&IpcResponse::Vault(reply.clone())).unwrap();
        let back: IpcResponse = serde_json::from_str(&text).unwrap();
        assert!(
            matches!(&back, IpcResponse::Vault(inner) if *inner == reply),
            "{text}"
        );
    }
    let last = serde_json::to_value(IpcResponse::Vault(VaultIpcReply::ItemPage {
        items: Vec::new(),
        next_offset: None,
    }))
    .unwrap();
    assert_eq!(last["kind"], "item_page");
    // The last page has no `next_offset`; the keys of an entry are the CLI's.
    assert!(last.get("next_offset").is_none());
    let value = serde_json::to_value(item()).unwrap();
    let mut keys: Vec<String> = value.as_object().unwrap().keys().cloned().collect();
    keys.sort();
    assert_eq!(
        keys,
        ["a", "actions", "b", "finding", "group", "item", "kind", "note", "path"]
    );
}
