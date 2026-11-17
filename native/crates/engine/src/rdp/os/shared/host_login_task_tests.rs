//! Task tests of the host side of the sign-in with the grant's account
//! (plan 2026-11-17 P4, decisions 2, 7 and 8): steering before and after,
//! the limit of wrong passwords and what the end of steering forgets.
use super::*;
use std::cell::Cell;

/// Decision 8: without „Bildschirm steuern“ nothing is issued; a right that
/// ends during the work withdraws what was issued and refuses. Decision 2:
/// at most five wrong passwords per viewer and hour.
#[test]
fn rdp_konto_task_login_refused_without_steering() {
    let refusal = "Remote Desktop und lokale Dienste brauchen „Bildschirm steuern: Erlauben“";
    let (worked, undone) = (Cell::new(false), Cell::new(false));
    let refused = with_steering(
        || Err(refusal.to_string()),
        || {
            worked.set(true);
            Ok(())
        },
        |_| undone.set(true),
    );
    assert_eq!(refused, Err(refusal.to_string()));
    assert!(!worked.get() && !undone.get());

    let calls = Cell::new(0);
    let ends_meanwhile = || {
        calls.set(calls.get() + 1);
        if calls.get() == 1 {
            Ok(())
        } else {
            Err(refusal.to_string())
        }
    };
    let issued = || {
        let account = HostAccount::new("se-remote", Some("PC-B")).unwrap();
        let secret = ManagedSecret::new("neu-je-Sitzung".into());
        Ok(LoginInfo::new(
            Some(account),
            Some(secret),
            LoginState::Automatic,
        ))
    };
    let result = with_steering(ends_meanwhile, issued, |info: &LoginInfo| {
        undone.set(info.secret.is_some())
    });
    assert!(result.is_err() && undone.get(), "{result:?}");
    assert_eq!(calls.get(), 2);

    undone.set(false);
    let kept = with_steering(|| Ok(()), issued, |_| undone.set(true)).unwrap();
    assert_eq!(kept.state, LoginState::Automatic);
    assert!(!undone.get());

    let key = "rdp-konto-task|failures";
    let start = Instant::now();
    assert_eq!(failures_left(key, start), MAX_FAILURES);
    for used in 1..=MAX_FAILURES {
        note_failure(key, start);
        assert_eq!(failures_left(key, start), MAX_FAILURES - used);
    }
    assert_eq!(failures_left("rdp-konto-task|other", start), MAX_FAILURES);
    let later = start + FAILURE_WINDOW + Duration::from_secs(1);
    assert_eq!(failures_left(key, later), MAX_FAILURES);
}

/// Decision 7: when „Bildschirm steuern“ ends for a viewer its stored
/// passwords go; those of viewers that still steer stay. The name in the
/// credential store carries no device data and differs per account.
#[test]
fn rdp_konto_task_stored_password_forgotten_when_steering_ends() {
    let entry = |device: &str, account: &str| StoredEntry {
        device_id: device.into(),
        public_key: format!("key-{device}"),
        account: account.into(),
        at: 1,
    };
    let entries = vec![
        entry("laptop", "PC-B\\alice"),
        entry("tablet", "PC-B\\alice"),
        entry("laptop", "PC-B\\bob"),
    ];
    let (kept, gone) = forget_unsteered(entries.clone(), |stored| stored.device_id == "tablet");
    assert_eq!(kept, vec![entries[1].clone()]);
    assert_eq!(gone, vec![entries[0].clone(), entries[2].clone()]);
    let (kept, gone) = forget_unsteered(entries.clone(), |_| false);
    assert!(kept.is_empty() && gone.len() == 3);

    let name = entries[0].credential();
    assert!(name.starts_with("smart-explorer:rdp-login:"), "{name}");
    for private in ["laptop", "key-laptop", "alice", "PC-B"] {
        assert!(!name.contains(private), "{name}");
    }
    assert_ne!(name, entries[2].credential());
    assert_ne!(name, entries[1].credential());
    assert_eq!(
        name,
        StoredEntry {
            at: 99,
            ..entries[0].clone()
        }
        .credential()
    );
    assert_eq!(
        Viewer::new("laptop", "key-laptop").binding(),
        "laptop|key-laptop"
    );
}
