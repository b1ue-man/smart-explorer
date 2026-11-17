//! Task tests of the xrdp sign-in with a one-time se token (plan 2026-11-17
//! P4, decision 6): the PAM line and the decision of the PAM helper.
use super::*;

const DEBIAN: &str = "#%PAM-1.0
auth required pam_env.so readenv=1
auth required pam_env.so readenv=1 envfile=/etc/default/locale
@include common-auth
-auth optional pam_gnome_keyring.so
-auth optional pam_kwallet5.so

@include common-account
@include common-session
-session optional pam_gnome_keyring.so auto_start
@include common-password
";

const FEDORA: &str = "#%PAM-1.0
auth       include      password-auth
account    include      password-auth
password   include      password-auth
session    include      password-auth
";

fn se_lines(text: &str) -> usize {
    text.lines()
        .filter(|line| line.split_whitespace().any(|word| word == PAM_SWITCH))
        .count()
}

/// The se line stands once, right before the shared password stack, behind
/// its comment; a second run changes nothing, another helper path replaces
/// it, an unknown layout stays untouched, and removing it restores the file.
#[test]
fn rdp_konto_task_xrdp_pam_line_inserted_once_before_common_auth() {
    let line = pam_line("/usr/local/bin/se").unwrap();
    assert_eq!(
        line,
        "auth [success=done default=ignore] pam_exec.so quiet expose_authtok type=auth /usr/local/bin/se --rdp-xrdp-login"
    );
    let (with, changed) = pam_with_line(DEBIAN, &line).unwrap();
    assert!(changed);
    let lines: Vec<&str> = with.lines().collect();
    let at = lines
        .iter()
        .position(|l| *l == "@include common-auth")
        .unwrap();
    assert_eq!(lines[at - 1], line);
    assert_eq!(lines[at - 2], PAM_COMMENT);
    assert!(lines[..at - 2].iter().all(|l| !l.contains("pam_exec")));
    assert_eq!(se_lines(&with), 1);
    assert_eq!(pam_with_line(&with, &line).unwrap(), (with.clone(), false));
    let moved = pam_line("/opt/smart-explorer/se").unwrap();
    let (replaced, changed) = pam_with_line(&with, &moved).unwrap();
    assert!(changed && replaced.contains(&moved) && !replaced.contains(&line));
    assert_eq!(se_lines(&replaced), 1);
    let (fedora, _) = pam_with_line(FEDORA, &line).unwrap();
    let lines: Vec<&str> = fedora.lines().collect();
    assert_eq!(lines[1], PAM_COMMENT);
    assert_eq!(lines[2], line);
    assert!(lines[3].starts_with("auth") && lines[3].contains("password-auth"));
    assert!(pam_with_line("auth required pam_unix.so\n", &line).is_err());
    let (without, changed) = pam_without_line(&with);
    assert!(changed);
    assert_eq!(without, DEBIAN);
    assert_eq!(pam_without_line(DEBIAN), (DEBIAN.to_string(), false));
    for unusable in ["se", "/usr/local/bin/s e", "/x]", "/x#y", "/x\ny"] {
        assert!(pam_line(unusable).is_err(), "{unusable:?}");
    }
}

/// A token signs in only the account it names, only before it expires and
/// only while a se tunnel of the worker connects to the xrdp listener; a
/// typed password is no token. A worker that is not root signs in its own
/// account and the accounts listed for it, never root.
#[test]
fn rdp_konto_task_xrdp_token_bound_to_user_and_tunnel() {
    let token = new_token().unwrap();
    assert!(token.starts_with(TOKEN_PREFIX) && token.len() > TOKEN_PREFIX.len() + 40);
    assert_ne!(token, new_token().unwrap());
    let record = TokenRecord {
        user: "alice".into(),
        digest: token_digest(&token),
        port: 3389,
        device_id: "viewer".into(),
        public_key: "key".into(),
        expires_at: 1_000,
    };
    assert!(!record.digest.contains(&token[TOKEN_PREFIX.len()..]));
    let records = vec![record.clone()];
    let live = |port: u16| port == 3389;
    assert!(token_accepted(&records, "alice", &token, 999, live));
    assert!(!token_accepted(&records, "bob", &token, 999, live));
    assert!(!token_accepted(
        &records,
        "alice",
        &new_token().unwrap(),
        999,
        live
    ));
    assert!(!token_accepted(&records, "alice", "Geheim-1", 999, live));
    assert!(!token_accepted(&records, "alice", &token, 1_000, live));
    assert!(!token_accepted(&records, "alice", &token, 999, |_| false));
    let other_port = vec![TokenRecord {
        port: 3390,
        ..record
    }];
    assert!(!token_accepted(&other_port, "alice", &token, 999, live));

    assert!(user_allowed(1000, Some(1000), false));
    assert!(!user_allowed(1000, Some(1001), false));
    assert!(user_allowed(1000, Some(1001), true));
    assert!(!user_allowed(1000, Some(0), true));
    assert!(!user_allowed(1000, None, true));
    assert!(user_allowed(0, Some(0), false));

    // Client side of the se tunnel: 127.0.0.1:41394 -> 127.0.0.1:3389 (0x0D3D).
    let row = "  3: 0100007F:A1B2 0100007F:0D3D 01 00000000:00000000 00:00000000 00000000  1000        0 12345 1 0000000000000000 20 4 30 10 -1";
    assert!(tunnel_row(row, 3389, 1000));
    assert!(!tunnel_row(row, 3389, 1001));
    assert!(!tunnel_row(row, 3390, 1000));
    assert!(!tunnel_row(&row.replace(" 01 ", " 0A "), 3389, 1000));
    assert!(!tunnel_row(
        &row.replace("0100007F:0D3D", "0101A8C0:0D3D"),
        3389,
        1000
    ));
    let six = "  0: 00000000000000000000000001000000:A1B2 00000000000000000000000001000000:0D3D 01 00000000:00000000 00:00000000 00000000     0        0 1 1 0 20 4 30 10 -1";
    assert!(tunnel_row(six, 3389, 0));
    let mapped = six.replace(
        "00000000000000000000000001000000:0D3D",
        "0000000000000000FFFF00000100007F:0D3D",
    );
    assert!(tunnel_row(&mapped, 3389, 0));
    assert!(!tunnel_row("garbage", 3389, 0));
}
