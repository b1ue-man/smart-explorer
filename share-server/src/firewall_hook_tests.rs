use super::*;
use crate::config::Command;

const EXE: &str = "/opt/smart-explorer/share-server/se-share-server";

fn strings(words: &[&str]) -> Vec<String> {
    words.iter().map(|word| word.to_string()).collect()
}

/// The drop-in comes from `systemctl show UNIT -p ExecStart --value` of a
/// running service (and of the ExecStartEx form with `flags=`): `+-`, the
/// unit's program, its arguments and `--admit-firewall`, after the firewall
/// services. A missing, foreign or doubled ExecStart is refused; `%` and words
/// with blanks, quotes or backslashes keep their meaning, `$` stays for systemd.
#[test]
fn firewall_hook_task_drop_in_from_exec_start() {
    let show = format!(
        "{{ path={EXE} ; argv[]={EXE} 0.0.0.0:51820 --tls-cert /etc/se-share/cert.pem --tls-key /etc/se-share/key.pem --state-file /var/lib/se-share/bindings.json ; ignore_errors=no ; start_time=[Tue 2026-11-24 10:00:00 UTC] ; stop_time=[n/a] ; pid=812 ; code=(null) ; status=0/0 }}\n"
    );
    let exec = parse_exec_start(&show).expect("ExecStart");
    assert_eq!(exec.path, EXE);
    assert_eq!(
        exec.args,
        [
            "0.0.0.0:51820",
            "--tls-cert",
            "/etc/se-share/cert.pem",
            "--tls-key",
            "/etc/se-share/key.pem",
            "--state-file",
            "/var/lib/se-share/bindings.json"
        ]
    );
    assert_eq!(
        drop_in_text(&exec.path, &exec.args),
        "# Written by se-share-server --install-firewall-hook, rewritten by it when ExecStart changes.\n\
         # \"+\": systemd runs the step as root, without User= and the sandbox of the service;\n\
         # \"-\": a failure never stops the start. The step admits the server's listening\n\
         # ports in the host firewall and exits 0.\n\
         [Unit]\n\
         After=ufw.service firewalld.service nftables.service netfilter-persistent.service iptables.service ip6tables.service\n\
         \n\
         [Service]\n\
         ExecStartPre=+-/opt/smart-explorer/share-server/se-share-server 0.0.0.0:51820 --tls-cert /etc/se-share/cert.pem --tls-key /etc/se-share/key.pem --state-file /var/lib/se-share/bindings.json --admit-firewall\n"
    );

    // Case: the ExecStartEx form; an --admit-firewall already there stays once.
    let ex = "{ path=/usr/local/bin/se-share-server ; argv[]=/usr/local/bin/se-share-server 127.0.0.1:51820 --admit-firewall ; flags= ; start_time=[n/a] ; stop_time=[n/a] ; pid=0 ; code=(null) ; status=0/0 }";
    let exec = parse_exec_start(ex).expect("ExecStartEx");
    assert_eq!(exec.args, ["127.0.0.1:51820"]);
    assert_eq!(
        pre_start_line(&exec.path, &exec.args),
        "ExecStartPre=+-/usr/local/bin/se-share-server 127.0.0.1:51820 --admit-firewall"
    );

    // Case: no unit (systemctl prints nothing), another program, two entries.
    assert!(parse_exec_start("\n")
        .expect_err("no ExecStart")
        .contains("kein ExecStart"));
    let cron = "{ path=/usr/sbin/cron ; argv[]=/usr/sbin/cron -f ; ignore_errors=no ; pid=0 }";
    assert!(parse_exec_start(cron)
        .expect_err("foreign program")
        .contains("nicht se-share-server"));
    assert!(parse_exec_start(&format!("{ex}\n{ex}\n")).is_err());

    // Case: systemd command-line words.
    assert_eq!(exec_word("50%"), "50%%");
    assert_eq!(
        exec_word("/srv/my certs/cert.pem"),
        "\"/srv/my certs/cert.pem\""
    );
    assert_eq!(exec_word(r#"a"b\c"#), r#""a\"b\\c""#);
    assert_eq!(exec_word(""), "\"\"");
    assert_eq!(exec_word(";"), "\";\"");
    assert_eq!(exec_word("$SE_SHARE_ARGS"), "$SE_SHARE_ARGS");
    assert_eq!(
        pre_start_line(
            "/opt/se-share-server",
            &strings(&["/srv/my certs/c.pem", "5%"])
        ),
        "ExecStartPre=+-/opt/se-share-server \"/srv/my certs/c.pem\" 5%% --admit-firewall"
    );
}

/// The installation writes the drop-in once: 0644 in a 0755 directory, no
/// rewrite (same inode) while the text is the same, a repaired mode without a
/// rewrite, a rewrite when ExecStart changed, no temporary file left behind.
#[cfg(target_os = "linux")]
#[test]
fn firewall_hook_task_drop_in_written_once() {
    use super::platform::write_drop_in;
    use std::os::unix::fs::{MetadataExt, PermissionsExt};
    let root = std::env::temp_dir().join(format!("se-firewall-hook-task-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    let dir = root.join("se-share-server.service.d");
    let path = dir.join(DROP_IN);
    let mode = |path: &std::path::PathBuf| {
        std::fs::metadata(path)
            .expect("metadata")
            .permissions()
            .mode()
            & 0o777
    };
    let inode = |path: &std::path::PathBuf| std::fs::metadata(path).expect("metadata").ino();

    let first = drop_in_text(EXE, &strings(&["0.0.0.0:51820"]));
    assert!(write_drop_in(&dir, &first).expect("first write"));
    assert_eq!(std::fs::read_to_string(&path).expect("drop-in"), first);
    assert_eq!((mode(&path), mode(&dir)), (0o644, 0o755));
    let written = inode(&path);

    assert!(!write_drop_in(&dir, &first).expect("same text"));
    assert_eq!(inode(&path), written, "the same text is not rewritten");

    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600)).expect("chmod");
    assert!(!write_drop_in(&dir, &first).expect("same text, other mode"));
    assert_eq!((mode(&path), inode(&path)), (0o644, written));

    let second = drop_in_text(EXE, &strings(&["0.0.0.0:51830"]));
    assert!(write_drop_in(&dir, &second).expect("changed ExecStart"));
    assert_eq!(std::fs::read_to_string(&path).expect("drop-in"), second);
    let names: Vec<String> = std::fs::read_dir(&dir)
        .expect("drop-in directory")
        .map(|entry| {
            entry
                .expect("entry")
                .file_name()
                .to_string_lossy()
                .into_owned()
        })
        .collect();
    assert_eq!(names, [DROP_IN]);
    std::fs::remove_dir_all(&root).expect("cleanup");
}

fn command(args: &[&str]) -> Result<Command, String> {
    Command::parse(strings(args), |_| Ok(None))
}

/// `--admit-firewall` runs on the server's own settings wherever it stands and
/// carries a settings error along (the step logs it and ends 0);
/// `--install-firewall-hook` takes only `--unit` (default
/// se-share-server.service, `.service` appended, systemd name characters);
/// `--unit` alone is an error and `--help` wins.
#[test]
fn firewall_hook_task_admit_firewall_arguments() {
    let tls = ["--tls-cert", "cert.pem", "--tls-key", "key.pem"];
    let mut args = vec!["--admit-firewall", "0.0.0.0:51830"];
    args.extend(tls);
    let Ok(Command::AdmitFirewall(Ok(options))) = command(&args) else {
        panic!("--admit-firewall with settings");
    };
    assert_eq!(options.bind.to_string(), "0.0.0.0:51830");
    assert!(options.cert.is_some() && options.key.is_some());
    let Ok(Command::AdmitFirewall(Ok(options))) = command(&["127.0.0.1:51820", "--admit-firewall"])
    else {
        panic!("--admit-firewall after the bind");
    };
    assert_eq!(options.bind.to_string(), "127.0.0.1:51820");
    assert!(options.cert.is_none());
    let Ok(Command::AdmitFirewall(Err(error))) = command(&["--admit-firewall", "--bogus"]) else {
        panic!("a settings error stays inside --admit-firewall");
    };
    assert!(error.contains("unknown argument --bogus"), "{error}");
    assert!(command(&["--bogus"]).is_err());
    assert!(matches!(command(&[]), Ok(Command::Serve(_))));
    assert!(matches!(
        command(&["--admit-firewall", "--help"]),
        Ok(Command::Help)
    ));

    let unit = |args: &[&str]| match command(args) {
        Ok(Command::InstallFirewallHook { unit }) => Ok(unit),
        Ok(_) => Err("another command".to_string()),
        Err(error) => Err(error),
    };
    assert_eq!(
        unit(&["--install-firewall-hook"]).as_deref(),
        Ok("se-share-server.service")
    );
    assert_eq!(
        unit(&["--install-firewall-hook", "--unit", "share"]).as_deref(),
        Ok("share.service")
    );
    assert_eq!(
        unit(&["--unit", "se-share@eu.service", "--install-firewall-hook"]).as_deref(),
        Ok("se-share@eu.service")
    );
    for (args, error) in [
        (
            &["--install-firewall-hook", "0.0.0.0:51820"][..],
            "takes only --unit",
        ),
        (&["--install-firewall-hook", "--unit"][..], "needs a value"),
        (
            &["--install-firewall-hook", "--unit", "../etc"][..],
            "not a systemd service name",
        ),
        (
            &["--install-firewall-hook", "--unit", "a", "--unit", "b"][..],
            "given twice",
        ),
        (&["--unit", "share"][..], "only goes with"),
    ] {
        let message = unit(args).expect_err("refused");
        assert!(message.contains(error), "{args:?}: {message}");
    }
}

/// A start without rights inside a system service logs one line with the
/// exact command that installs the step while the drop-in is missing or
/// carries other arguments; a current drop-in (or one with a `$` variable)
/// takes over. The unit comes from `/proc/self/cgroup`; a user manager, a
/// scope and a container root name none.
#[test]
fn firewall_hook_task_missing_hook_log_line() {
    let unit = "se-share-server.service";
    let args = strings(&["0.0.0.0:51820", "--state-file", "/var/lib/se-share/b.json"]);
    let command = install_command(EXE, unit);
    assert_eq!(
        command,
        "sudo /opt/smart-explorer/share-server/se-share-server --install-firewall-hook"
    );
    assert_eq!(hook_state(None, EXE, &args), HookState::Missing);
    assert_eq!(
        hint(HookState::Missing, unit, &command).as_deref(),
        Some(
            "se-share-server.service läuft ohne Root-Rechte und ohne Vorstart-Schritt; einmal als root einrichten, danach gibt systemd die Ports vor jedem Start frei: sudo /opt/smart-explorer/share-server/se-share-server --install-firewall-hook"
        )
    );

    let current = drop_in_text(EXE, &args);
    assert_eq!(hook_state(Some(&current), EXE, &args), HookState::Current);
    assert_eq!(hint(HookState::Current, unit, &command), None);
    let variable = drop_in_text(EXE, &strings(&["$SE_SHARE_ARGS"]));
    assert_eq!(hook_state(Some(&variable), EXE, &args), HookState::Current);

    let older = drop_in_text(EXE, &strings(&["0.0.0.0:51820"]));
    assert_eq!(hook_state(Some(&older), EXE, &args), HookState::Stale);
    assert_eq!(
        hook_state(Some("[Service]\n"), EXE, &args),
        HookState::Stale
    );
    let stale = hint(HookState::Stale, unit, &command).expect("stale line");
    assert!(stale.contains("/etc/systemd/system/se-share-server.service.d/se-firewall.conf"));
    assert!(stale.ends_with(&command), "{stale}");

    assert_eq!(
        install_command("/srv/se share/se-share-server", "share.service"),
        "sudo '/srv/se share/se-share-server' --install-firewall-hook --unit share.service"
    );

    for (cgroup, unit) in [
        ("0::/system.slice/se-share-server.service\n", Some(unit)),
        (
            "12:pids:/system.slice/x.service\n1:name=systemd:/system.slice/se-share-server.service\n",
            Some(unit),
        ),
        ("0::/system.slice/se-share-server.service/payload\n", Some(unit)),
        (
            "0::/user.slice/user-1000.slice/user@1000.service/app.slice/se.service\n",
            None,
        ),
        ("0::/user.slice/user-1000.slice/session-3.scope\n", None),
        ("0::/\n", None),
    ] {
        assert_eq!(unit_from_cgroup(cgroup).as_deref(), unit, "{cgroup}");
    }
    assert_eq!(
        nul_fields(b"/opt/se-share-server\x000.0.0.0:51820\x00"),
        ["/opt/se-share-server", "0.0.0.0:51820"]
    );
}
