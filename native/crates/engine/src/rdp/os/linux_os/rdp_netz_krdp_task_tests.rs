//! E29 (M11): a KRdp that se sets up starts with `--address 127.0.0.1`. The argv
//! rewrite, the unit text and the drop-in text are checked as fixed text; no
//! systemd call runs in these tests.
use super::{
    all_interfaces_notes, dropin_text, exec_start_from_unit_text, loopback_argv, parse_exec_start,
};

fn argv(parts: &[&str]) -> Vec<String> {
    parts.iter().map(|part| part.to_string()).collect()
}

/// Other arguments keep their order; the loopback pair ends the argv; a second
/// application changes nothing.
#[test]
fn rdp_netz_task_krdp_argument_is_loopback_and_keeps_other_arguments() {
    let configured = argv(&[
        "/usr/bin/krdpserver",
        "--port",
        "3389",
        "--mode",
        "SharedAccess",
    ]);
    let wanted = loopback_argv(&configured);
    assert_eq!(
        wanted,
        argv(&[
            "/usr/bin/krdpserver",
            "--port",
            "3389",
            "--mode",
            "SharedAccess",
            "--address",
            "127.0.0.1",
        ])
    );
    assert_eq!(loopback_argv(&wanted), wanted);
}

/// An address given as `0.0.0.0` in either form is replaced, never kept.
#[test]
fn rdp_netz_task_krdp_argument_replaces_any_address_form() {
    let separate = loopback_argv(&argv(&[
        "/usr/bin/krdpserver",
        "--address",
        "0.0.0.0",
        "--port",
        "3389",
    ]));
    assert_eq!(
        separate,
        argv(&[
            "/usr/bin/krdpserver",
            "--port",
            "3389",
            "--address",
            "127.0.0.1",
        ])
    );
    let joined = loopback_argv(&argv(&["/usr/bin/krdpserver", "--address=0.0.0.0"]));
    assert_eq!(
        joined,
        argv(&["/usr/bin/krdpserver", "--address", "127.0.0.1"])
    );
}

/// The argv is read from the `argv[]=` part of the `systemctl show` output,
/// up to the next field and up to a closing brace.
#[test]
fn rdp_netz_task_krdp_exec_start_is_read_from_show_output() {
    let show = "ExecStart={ path=/usr/bin/krdpserver ; argv[]=/usr/bin/krdpserver --port 3389 ; ignore_errors=no ; start_time=[n/a] ; stop_time=[n/a] ; pid=0 ; code=(null) ; status=0/0 }";
    assert_eq!(
        parse_exec_start(show).expect("argv of the show output"),
        argv(&["/usr/bin/krdpserver", "--port", "3389"])
    );
    let last_field = "ExecStart={ path=/usr/bin/krdpserver ; argv[]=/usr/bin/krdpserver }";
    assert_eq!(
        parse_exec_start(last_field).expect("argv as the last field"),
        argv(&["/usr/bin/krdpserver"])
    );
    assert!(parse_exec_start("ExecStart={ path=/usr/bin/krdpserver ; pid=0 }").is_err());
}

/// The drop-in clears the unit's command first, then sets the whole argv.
#[test]
fn rdp_netz_task_krdp_drop_in_clears_then_sets_the_loopback_command() {
    let text = dropin_text(&argv(&[
        "/usr/bin/krdpserver",
        "--port",
        "3389",
        "--address",
        "127.0.0.1",
    ]))
    .expect("plain tokens");
    assert_eq!(
        text,
        "[Service]\nExecStart=\nExecStart=/usr/bin/krdpserver --port 3389 --address 127.0.0.1\n"
    );
}

/// An argument that systemd would read differently (spaces, quotes,
/// specifiers, an empty token) is refused; se never rewrites it silently.
#[test]
fn rdp_netz_task_krdp_drop_in_refuses_arguments_systemd_would_change() {
    assert!(dropin_text(&argv(&[
        "/usr/bin/krdpserver",
        "--certificate",
        "/tmp/a b.crt",
    ]))
    .is_err());
    assert!(dropin_text(&argv(&["/usr/bin/krdpserver", "--note", "$HOME"])).is_err());
    assert!(dropin_text(&argv(&["/usr/bin/krdpserver", ""])).is_err());
}

/// O7: the unit text gives the argument boundaries of the effective command; a
/// later `ExecStart=` after an empty reset is the one that counts.
#[test]
fn rdp_netz_task_krdp_exec_start_from_unit_text_keeps_the_boundaries() {
    let text = "# /usr/lib/systemd/user/app-org.kde.krdpserver.service\n[Service]\nExecStart=/usr/bin/krdpserver --port 3389 --mode SharedAccess\n\n# /home/example/.config/systemd/user/app-org.kde.krdpserver.service.d/a.conf\n[Service]\nExecStart=\nExecStart=/usr/bin/krdpserver --port 3390\n";
    assert_eq!(
        exec_start_from_unit_text(text).expect("one plain command"),
        argv(&["/usr/bin/krdpserver", "--port", "3390"])
    );
}

/// O7: an argument with a space is quoted in the unit text and is refused, so no
/// drop-in is written from words that the show output split apart. The show
/// output of the same command is shown as the flattened form it produces.
#[test]
fn rdp_netz_task_krdp_argument_with_whitespace_is_refused_not_split() {
    let quoted = "[Service]\nExecStart=/usr/bin/krdpserver --certificate \"/opt/a b/cert.pem\"\n";
    assert!(exec_start_from_unit_text(quoted).is_err());
    let flattened = "ExecStart={ path=/usr/bin/krdpserver ; argv[]=/usr/bin/krdpserver --certificate /opt/a b/cert.pem ; ignore_errors=no ; pid=0 }";
    assert_eq!(
        parse_exec_start(flattened).expect("argv of the show output"),
        argv(&[
            "/usr/bin/krdpserver",
            "--certificate",
            "/opt/a",
            "b/cert.pem",
        ])
    );
}

/// O7: a command prefix, two commands or a unit without a command are refused.
#[test]
fn rdp_netz_task_krdp_unit_text_with_prefix_or_two_commands_is_refused() {
    assert!(exec_start_from_unit_text("[Service]\nExecStart=-/usr/bin/krdpserver\n").is_err());
    assert!(
        exec_start_from_unit_text("[Service]\nExecStart=/usr/bin/a\nExecStart=/usr/bin/b\n")
            .is_err()
    );
    assert!(exec_start_from_unit_text("[Service]\nDescription=none\n").is_err());
    assert!(
        exec_start_from_unit_text("[Service]\nExecStart=/usr/bin/krdpserver --note $HOME\n")
            .is_err()
    );
}

/// E56: a running KRdp whose command names no loopback address gives the evidence
/// and the diagnosis line. A command bound to 127.0.0.1 (either form) gives none,
/// a stopped unit or one without a known command gives none, and a later
/// non-loopback address still counts.
#[test]
fn rdp_netz_krdp_task_unit_without_loopback_address_gives_evidence_and_note() {
    let running = argv(&["/usr/bin/krdpserver", "--port", "3389"]);
    let (evidence, note) =
        all_interfaces_notes(true, &running).expect("a running KRdp without a loopback address");
    assert_eq!(
        evidence,
        "KRdp lauscht auf allen Schnittstellen (nicht von se auf Loopback gestellt)"
    );
    assert!(note.contains("aus dem lokalen Netz erreichbar"));
    for loopback in [
        argv(&[
            "/usr/bin/krdpserver",
            "--port",
            "3389",
            "--address",
            "127.0.0.1",
        ]),
        argv(&["/usr/bin/krdpserver", "--address=127.0.0.1"]),
        // Every loopback address only listens locally (no false network claim).
        argv(&["/usr/bin/krdpserver", "--address", "::1"]),
        argv(&["/usr/bin/krdpserver", "--address=127.0.0.2"]),
    ] {
        assert!(all_interfaces_notes(true, &loopback).is_none());
    }
    assert!(all_interfaces_notes(false, &argv(&["/usr/bin/krdpserver"])).is_none());
    assert!(all_interfaces_notes(true, &[]).is_none());
    let mixed = argv(&[
        "/usr/bin/krdpserver",
        "--address",
        "127.0.0.1",
        "--address",
        "0.0.0.0",
    ]);
    assert!(all_interfaces_notes(true, &mixed).is_some());
}
