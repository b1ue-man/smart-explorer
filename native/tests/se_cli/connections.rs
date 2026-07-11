use std::fs;
#[cfg(target_os = "linux")]
use std::io::{BufRead, BufReader, Write};
#[cfg(target_os = "linux")]
use std::net::{TcpListener, TcpStream};
#[cfg(target_os = "linux")]
use std::os::unix::fs::PermissionsExt;
#[cfg(target_os = "linux")]
use std::thread;
use std::time::Duration;
#[cfg(target_os = "linux")]
use std::time::Instant;

use crate::support::{
    assert_exit_code, assert_success, collect_bounded, contains_case_insensitive, run,
    spawn_captured, stderr, stdout, Sandbox,
};
#[cfg(target_os = "linux")]
use crate::support::{assert_output_omits, run_bounded, run_with_stdin_bounded};

#[test]
fn saved_connections_persist_across_processes_and_sandboxes_are_isolated() {
    let sandbox = Sandbox::new("connection-persistence");
    let isolated = Sandbox::new("connection-isolation");

    let added = run(sandbox
        .command()
        .args(["connections", "add", "sftp"])
        .args(["--host", "metadata.invalid"])
        .args(["--port", "2222"])
        .args(["--user", "alice"])
        .args(["--root", "/srv"])
        .args(["--label", "metadata-only"]));
    assert_success(&added);

    let listed = run(sandbox.command().args(["connections", "list", "--json"]));
    assert_success(&listed);
    let rows: serde_json::Value =
        serde_json::from_slice(&listed.stdout).expect("connections JSON must parse");
    let rows = rows.as_array().expect("connections JSON must be an array");
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0]["label"], "metadata-only");
    assert_eq!(rows[0]["host"], "metadata.invalid");

    let isolated_list = run(isolated.command().args(["connections", "list", "--json"]));
    assert_success(&isolated_list);
    assert_eq!(stdout(&isolated_list).trim(), "[]");
}

#[test]
fn malformed_metadata_rejects_add_without_rewriting_the_file() {
    let sandbox = Sandbox::new("connection-corruption");
    let path = sandbox.app_data_path("connections.txt");
    fs::create_dir_all(path.parent().expect("connections parent"))
        .expect("create connections parent");
    let original = b"not-a-valid-connection\n";
    fs::write(&path, original).expect("write malformed connections fixture");

    let output = run(sandbox
        .command()
        .args(["connections", "add", "sftp"])
        .args(["--host", "must-not-save.invalid"])
        .args(["--label", "must-not-save"]));

    assert_exit_code(&output, 1);
    assert!(output.stdout.is_empty());
    assert!(stderr(&output).starts_with("se: "));
    assert!(
        contains_case_insensitive(stderr(&output), "metadata")
            || contains_case_insensitive(stderr(&output), "metadaten")
    );
    assert_eq!(
        fs::read(&path).expect("re-read malformed metadata"),
        original
    );
}

#[test]
fn concurrent_connection_adds_retain_every_entry() {
    let sandbox = Sandbox::new("connection-concurrency");
    let mut children = Vec::new();
    for index in 0..4 {
        let host = format!("concurrent-{index}.invalid");
        let label = format!("concurrent-{index}");
        let mut command = sandbox.command();
        command
            .args(["connections", "add", "sftp"])
            .args(["--host", &host])
            .args(["--port", "2222"])
            .args(["--user", "runner"])
            .args(["--root", "/srv"])
            .args(["--label", &label]);
        children.push(spawn_captured(&mut command));
    }

    for child in children {
        let bounded = collect_bounded(child, Duration::from_secs(5));
        assert!(!bounded.timed_out, "concurrent connection add timed out");
        assert_success(&bounded.output);
    }

    let listed = run(sandbox.command().args(["connections", "list", "--json"]));
    assert_success(&listed);
    let rows: serde_json::Value =
        serde_json::from_slice(&listed.stdout).expect("concurrent connections JSON must parse");
    let rows = rows.as_array().expect("connections JSON must be an array");
    assert_eq!(rows.len(), 4);
    for index in 0..4 {
        let label = format!("concurrent-{index}");
        assert!(rows
            .iter()
            .any(|row| row["label"].as_str() == Some(label.as_str())));
    }
}

#[cfg(target_os = "linux")]
#[test]
fn headless_secret_survives_processes_authenticates_and_is_removed() {
    let sandbox = Sandbox::new("headless-secret");
    let secret = "p:a ss%@word";
    let (port, server) = spawn_fake_ftp(2);
    let port = port.to_string();

    let added = run_with_stdin_bounded(
        sandbox
            .command()
            .args(["connections", "add", "ftp"])
            .args(["--host", "127.0.0.1"])
            .args(["--port", &port])
            .args(["--user", "headless"])
            .args(["--root", "/"])
            .args(["--label", "headless-ftp"])
            .arg("--password-stdin"),
        format!("{secret}\n").as_bytes(),
        Duration::from_secs(5),
    );
    assert!(!added.timed_out, "headless credential add timed out");
    assert_output_omits(&added.output, secret);
    assert_success(&added.output);

    let listed = run(sandbox.command().args(["connections", "list", "--json"]));
    assert_output_omits(&listed, secret);
    assert_success(&listed);
    assert!(stdout(&listed).contains("headless-ftp"));
    assert_metadata_omits(&sandbox, secret);
    let record = assert_secure_secret_record(&sandbox, &port);

    let doctor = run(sandbox.command().args(["doctor", "--json"]));
    assert_output_omits(&doctor, secret);
    assert_success(&doctor);

    let first_stat = run_bounded(
        sandbox.command().arg("stat").arg("@headless-ftp:/"),
        Duration::from_secs(8),
    );
    assert!(!first_stat.timed_out, "saved FTP authentication timed out");
    assert_output_omits(&first_stat.output, secret);
    assert_success(&first_stat.output);

    let removed = run(sandbox
        .command()
        .args(["connections", "remove", "headless-ftp"]));
    assert_output_omits(&removed, secret);
    assert_success(&removed);
    assert!(
        !record.exists(),
        "connection removal left its secret record"
    );
    let after_remove = run(sandbox.command().args(["connections", "list", "--json"]));
    assert_output_omits(&after_remove, secret);
    assert_success(&after_remove);
    assert_eq!(stdout(&after_remove).trim(), "[]");

    let readded = run(sandbox
        .command()
        .args(["connections", "add", "ftp"])
        .args(["--host", "127.0.0.1"])
        .args(["--port", &port])
        .args(["--user", "headless"])
        .args(["--root", "/"])
        .args(["--label", "headless-ftp"]));
    assert_output_omits(&readded, secret);
    assert_success(&readded);

    let second_stat = run_bounded(
        sandbox.command().arg("stat").arg("@headless-ftp:/"),
        Duration::from_secs(8),
    );
    assert!(
        !second_stat.timed_out,
        "credential-removal FTP check timed out"
    );
    assert_output_omits(&second_stat.output, secret);
    assert_success(&second_stat.output);

    let passwords = server.join().expect("join fake FTP server");
    assert_eq!(passwords.len(), 2);
    assert!(
        passwords[0].as_bytes() == secret.as_bytes(),
        "FTP did not receive the exact saved password bytes"
    );
    assert!(passwords[1].is_empty(), "removed FTP password was reused");

    let final_remove = run(sandbox
        .command()
        .args(["connections", "remove", "headless-ftp"]));
    assert_output_omits(&final_remove, secret);
    assert_success(&final_remove);
    assert!(!record.exists(), "final cleanup left its secret record");
    let final_list = run(sandbox.command().args(["connections", "list", "--json"]));
    assert_output_omits(&final_list, secret);
    assert_success(&final_list);
    assert_eq!(stdout(&final_list).trim(), "[]");
}

#[cfg(target_os = "linux")]
fn assert_metadata_omits(sandbox: &Sandbox, secret: &str) {
    for name in [
        "connections.txt",
        "share_profiles.json",
        "share_identity.json",
    ] {
        let path = sandbox.app_data_path(name);
        let Ok(bytes) = fs::read(path) else {
            continue;
        };
        assert!(
            !bytes
                .windows(secret.len())
                .any(|part| part == secret.as_bytes()),
            "non-secret application metadata contains credential bytes"
        );
    }
}

#[cfg(target_os = "linux")]
fn assert_secure_secret_record(sandbox: &Sandbox, port: &str) -> std::path::PathBuf {
    let directory = sandbox.app_data_path("secrets-v1");
    let directory_metadata = fs::symlink_metadata(&directory)
        .expect("inspect Linux credential directory from executable test");
    assert!(directory_metadata.file_type().is_dir());
    assert_eq!(directory_metadata.permissions().mode() & 0o7777, 0o700);

    let records: Vec<_> = fs::read_dir(&directory)
        .expect("read Linux credential directory")
        .map(|entry| entry.expect("read Linux credential entry").path())
        .filter(|path| {
            path.extension()
                .is_some_and(|extension| extension == "secret")
        })
        .collect();
    assert_eq!(records.len(), 1, "expected exactly one credential record");
    let record = records.into_iter().next().expect("one credential record");
    let name = record
        .file_name()
        .and_then(|name| name.to_str())
        .expect("credential record name must be Unicode");
    let digest = name
        .strip_suffix(".secret")
        .expect("credential record suffix");
    assert_eq!(digest.len(), 64);
    assert!(
        digest
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte)),
        "credential record name is not a lowercase SHA-256 digest"
    );

    let metadata = fs::symlink_metadata(&record).expect("inspect Linux credential record");
    assert!(metadata.file_type().is_file());
    assert_eq!(metadata.permissions().mode() & 0o7777, 0o600);
    assert!(metadata.len() > 32 && metadata.len() <= 66 * 1024);
    let bytes = fs::read(&record).expect("read Linux credential record envelope");
    assert!(
        serde_json::from_slice::<serde_json::Value>(&bytes).is_err(),
        "credential record must not be a JSON account map"
    );
    let account = format!("ftp://headless@127.0.0.1:{port}/");
    for account_text in [account.as_str(), "127.0.0.1", "headless-ftp"] {
        assert!(
            !name.contains(account_text)
                && !bytes
                    .windows(account_text.len())
                    .any(|part| part == account_text.as_bytes()),
            "credential record exposes account metadata"
        );
    }
    record
}

#[cfg(target_os = "linux")]
fn spawn_fake_ftp(connection_count: usize) -> (u16, thread::JoinHandle<Vec<String>>) {
    let listener = TcpListener::bind(("127.0.0.1", 0)).expect("bind fake FTP listener");
    let port = listener.local_addr().expect("read fake FTP address").port();
    listener
        .set_nonblocking(true)
        .expect("make fake FTP listener bounded");
    let handle = thread::spawn(move || {
        let deadline = Instant::now() + Duration::from_secs(15);
        let mut passwords = Vec::new();
        while passwords.len() < connection_count && Instant::now() < deadline {
            match listener.accept() {
                Ok((stream, _)) => passwords.push(handle_ftp_connection(stream)),
                Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                    thread::sleep(Duration::from_millis(10));
                }
                Err(error) => panic!("accept fake FTP connection: {error}"),
            }
        }
        assert_eq!(
            passwords.len(),
            connection_count,
            "fake FTP server did not receive every expected connection"
        );
        passwords
    });
    (port, handle)
}

#[cfg(target_os = "linux")]
fn handle_ftp_connection(mut stream: TcpStream) -> String {
    stream
        .set_read_timeout(Some(Duration::from_secs(5)))
        .expect("bound fake FTP reads");
    stream
        .set_write_timeout(Some(Duration::from_secs(5)))
        .expect("bound fake FTP writes");
    stream
        .write_all(b"220 Smart Explorer test FTP ready\r\n")
        .expect("write fake FTP greeting");
    let mut reader = BufReader::new(stream.try_clone().expect("clone fake FTP stream"));
    let mut password = None;
    loop {
        let mut line = String::new();
        let read = reader.read_line(&mut line).expect("read fake FTP command");
        assert!(read > 0, "FTP client disconnected before TYPE");
        let command = line.trim_end_matches(['\r', '\n']);
        let response = if command.starts_with("USER ") {
            "331 Password required\r\n"
        } else if command == "PASS" || command.starts_with("PASS ") {
            password = Some(
                command
                    .strip_prefix("PASS ")
                    .unwrap_or_default()
                    .to_string(),
            );
            "230 Login successful\r\n"
        } else if command == "TYPE I" {
            "200 Binary mode\r\n"
        } else {
            "200 Command accepted\r\n"
        };
        stream
            .write_all(response.as_bytes())
            .expect("write fake FTP response");
        if command == "TYPE I" {
            return password.expect("FTP client must authenticate before TYPE");
        }
    }
}
