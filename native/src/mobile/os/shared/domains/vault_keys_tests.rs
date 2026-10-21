//! VLT-M11 (S1): no password in cleartext in the secret store of the app,
//! only the envelope of the app; the memory scrypt may take.
use super::*;

#[test]
fn vault_task_android_no_plaintext_password_stored() {
    let refused = NoPlaintextStore.set("smart-explorer:vault:x", "Tresor-Passwort-1");
    assert_eq!(refused.unwrap_err(), NO_PLAINTEXT);
    let body = "A".repeat(38);
    assert!(is_envelope(&format!("android-kw1:{body}")));
    assert!(is_envelope(&format!("android-kw1:{body}-_09az")));
    for bad in [
        "Tresor-Passwort-1".to_string(),
        format!("android-kw1:{}", "A".repeat(37)),
        format!("android-kw2:{body}"),
        format!("android-kw1:{body}="),
        format!("android-kw1:{body} "),
    ] {
        assert!(!is_envelope(&bad), "{bad}");
    }
}

#[test]
fn vault_task_android_unlock_memory_from_app() {
    let args =
        serde_json::json!({ "memAvail": 2_000_000_000_i64, "memThreshold": 500_000_000_i64 });
    assert_eq!(memory(&args), Some(1_500_000_000));
    let below = serde_json::json!({ "memAvail": 100, "memThreshold": 500 });
    assert_eq!(memory(&below), Some(0));
}
