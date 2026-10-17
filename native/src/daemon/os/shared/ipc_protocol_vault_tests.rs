//! Wire form of `IpcRequest::Vault` and of `IpcResponse::Err.code` (stage
//! `vault_unit`).
use serde::Deserialize;

use super::super::{IpcRequest, IpcResponse};
use super::{SealPurposeWire, SecretText, VaultIpcReply, VaultIpcRequest, VaultStatusView};

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
    ]
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
    // Only the status of se Desktop leaves a drive-restore worker restricted.
    for request in every_request() {
        assert_eq!(
            request.promotes_daemon(),
            request != VaultIpcRequest::Status
        );
    }
}
