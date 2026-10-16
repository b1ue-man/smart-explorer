//! Gap-closing batch (2026-10-16), FTP on suppaftp 10.0.2 (RUSTSEC-2026-0271):
//! no command argument with CR, LF or NUL reaches the control channel, not
//! even the server's own PWD reply; an ABOR after a finished transfer leaves
//! no queued reply for the next command; refused listings free the data
//! channel; LIST rows keep the POSIX and DOS parsers. Against the scripted
//! loopback server of luecken_task_ftp_fixture.rs.
use super::backend_from_url;
use super::core_impl::parse_list_line_unchecked;
use super::errors::map;
use super::luecken_task_ftp_fixture::{Script, Server, ENDLESS};
use crate::vfs::Backend;
use std::io::{self, Read, Write};
use std::sync::atomic::AtomicBool;
use suppaftp::Status;

const BAD: [&str; 4] = [
    "/a\r\nDELE /victim",
    "/a\nDELE /victim",
    "/a\rDELE /victim",
    "/a\0DELE /victim",
];

fn no_delimiters(commands: &[String]) -> bool {
    commands
        .iter()
        .all(|command| !command.contains(['\r', '\0']) && !command.starts_with("DELE /victim"))
}

#[test]
fn luecken_task_ftp_delimiters_in_arguments_never_reach_the_server() {
    let server = Server::start(Script::default(), &[("/victim", &b"keep"[..])]);
    let backend = backend_from_url(&server.url()).unwrap();
    let extensions = backend.extensions().expect("FTP backend extensions");
    for bad in BAD {
        let write = |path: &str| {
            let mut writer = backend.open_write(path)?;
            writer.write_all(b"x")?;
            writer.flush()
        };
        let refused = [
            ("stat", backend.stat(bad).is_err()),
            ("list_dir", backend.list_dir(bad).is_err()),
            ("list_dir_for_sync", backend.list_dir_for_sync(bad).is_err()),
            ("open_read", backend.open_read(bad).is_err()),
            ("open_read_at", backend.open_read_at(bad, None, 3).is_err()),
            ("open_write", write(bad).is_err()),
            (
                "open_write_copy_stage_sized",
                backend.open_write_copy_stage_sized(bad, 1).is_err(),
            ),
            ("remove_file", backend.remove_file(bad).is_err()),
            ("remove_dir", backend.remove_dir(bad).is_err()),
            ("create_dir", backend.create_dir(bad).is_err()),
            ("create_dir_new", backend.create_dir_new(bad).is_err()),
            ("mkdir_all", backend.mkdir_all(bad).is_err()),
            ("rename from", backend.rename(bad, "/x").is_err()),
            ("rename to", backend.rename("/victim", bad).is_err()),
            ("promote_staged", backend.promote_staged(bad, "/y").is_err()),
            (
                "promote_staged_no_replace",
                backend.promote_staged_no_replace("/victim", bad).is_err(),
            ),
            (
                "remove_tree",
                extensions
                    .remove_tree(bad, &AtomicBool::new(false), &mut |_| {})
                    .is_err(),
            ),
        ];
        let accepted: Vec<&str> = refused
            .iter()
            .filter(|(_, refused)| !refused)
            .map(|(operation, _)| *operation)
            .collect();
        assert!(accepted.is_empty(), "{bad:?} accepted by {accepted:?}");
    }
    // The linked suppaftp refuses CR and LF itself (fixed in 10.0.2).
    let primary = backend.pool.primary();
    for injected in [
        primary.with_stream_mutation(|stream| stream.rm(BAD[0]).map_err(map)),
        primary.with_stream_mutation(|stream| {
            let command = "NOOP\r\nDELE /victim";
            stream
                .custom_command(command, &[Status::CommandOk])
                .map(|_| ())
                .map_err(map)
        }),
    ] {
        assert_eq!(injected.unwrap_err().kind(), io::ErrorKind::InvalidInput);
    }
    let commands = server.commands();
    assert!(no_delimiters(&commands), "{commands:?}");
    assert_eq!(server.file("/victim").as_deref(), Some(&b"keep"[..]));
}

#[test]
fn luecken_task_ftp_pwd_reply_with_a_delimiter_is_never_sent_back() {
    for pwd in [
        "257 \"/home\0x\" is current directory",
        "257 \"/home\rDELE /victim\" is current directory",
    ] {
        let script = Script {
            pwd: pwd.to_string(),
            ..Script::default()
        };
        let server = Server::start(
            script,
            &[("/victim", &b"keep"[..]), ("/dir/a.txt", &b"a"[..])],
        );
        let backend = backend_from_url(&server.url()).unwrap();
        let listed = backend.list_dir("/dir").map(|_| ());
        assert_eq!(listed.unwrap_err().kind(), io::ErrorKind::InvalidFilename);
        assert!(backend.mkdir_all("/dir/new").is_err());
        let extensions = backend.extensions().expect("FTP backend extensions");
        let cancel = AtomicBool::new(false);
        assert!(extensions
            .remove_tree("/dir", &cancel, &mut |_| {})
            .is_err());
        let commands = server.commands();
        // The folder is never entered with a PWD that could not be restored.
        let changed = ["CWD", "MKD", "DELE", "RMD"];
        assert!(
            !commands
                .iter()
                .any(|command| changed.iter().any(|verb| command.starts_with(verb))),
            "{pwd:?}: {commands:?}"
        );
        assert!(no_delimiters(&commands), "{pwd:?}: {commands:?}");
        assert!(server.file("/dir/a.txt").is_some() && server.file("/victim").is_some());
    }
}

#[test]
fn luecken_task_ftp_abort_after_a_finished_transfer_reconnects_before_reuse() {
    let server = Server::start(
        Script::default(),
        &[("/data.bin", &[7u8; 16][..]), ("/victim", &b"x"[..])],
    );
    let connection = server.connection();
    let mut reader = connection.open_reader("/data.bin").unwrap();
    let mut first = [0u8; 1];
    assert_eq!(reader.read(&mut first).unwrap(), 1);
    // ABOR after the server's 226: its own 226 for ABOR is still queued.
    drop(reader);
    let removed = connection.with_stream_mutation(|stream| stream.rm("/victim").map_err(map));
    assert!(removed.is_ok(), "{removed:?}: {:?}", server.commands());
    assert!(server.file("/victim").is_none());
    assert_eq!(server.accepts(), 2, "the connection was reused out of step");
}

#[test]
fn luecken_task_ftp_abort_during_a_transfer_keeps_the_connection() {
    let server = Server::start(Script::default(), &[("/victim", &b"x"[..])]);
    let connection = server.connection();
    let mut reader = connection.open_reader(ENDLESS).unwrap();
    let mut first = [0u8; 1];
    assert_eq!(reader.read(&mut first).unwrap(), 1);
    // ABOR while the server still sends: 426 for RETR, then 226 for ABOR.
    drop(reader);
    let removed = connection.with_stream_mutation(|stream| stream.rm("/victim").map_err(map));
    assert!(removed.is_ok(), "{removed:?}: {:?}", server.commands());
    assert!(server.file("/victim").is_none());
    assert_eq!(
        server.accepts(),
        1,
        "an aborted transfer lost its connection"
    );
}

#[test]
fn luecken_task_ftp_refused_listings_free_the_data_channel() {
    let script = Script {
        refuse_listings: true,
        ..Script::default()
    };
    let server = Server::start(script, &[("/a.txt", &b"abc"[..])]);
    let backend = backend_from_url(&server.url()).unwrap();
    let names: Vec<String> = backend
        .list_dir("/")
        .unwrap()
        .into_iter()
        .map(|meta| meta.name)
        .collect();
    assert_eq!(names, ["a.txt"]);
    let commands = server.commands();
    for expected in ["MLSD /", "LIST -a", "LIST"] {
        assert!(commands.iter().any(|seen| seen == expected), "{commands:?}");
    }
    // MLSD and LIST -a were refused on the same connection, LIST still ran.
    assert_eq!(server.accepts(), 1);
    let mut body = Vec::new();
    backend
        .open_read("/a.txt")
        .unwrap()
        .read_to_end(&mut body)
        .unwrap();
    assert_eq!(body, b"abc");
}

#[test]
fn luecken_task_ftp_list_rows_use_only_the_posix_and_dos_parsers() {
    let posix = parse_list_line_unchecked("-rw-r--r-- 1 0 0 5 Jan 1 2020 a.txt").unwrap();
    assert_eq!(
        (posix.name.as_str(), posix.size, posix.is_dir),
        ("a.txt", 5, false)
    );
    let dos = parse_list_line_unchecked("04-08-14  03:09PM  403 readme.txt").unwrap();
    assert_eq!(
        (dos.name.as_str(), dos.size, dos.is_dir),
        ("readme.txt", 403, false)
    );
    let folder = parse_list_line_unchecked("04-08-14  03:09PM  <DIR> pub").unwrap();
    assert!(folder.is_dir && folder.name == "pub");
    // suppaftp 10's `File::from_str` would also try MLSx, which takes any
    // text as a file name: an unreadable LIST row must stay an error.
    for row in [
        "type=file;size=5;modify=20200101000000; a.txt",
        "this server row is not parseable",
    ] {
        let parsed = parse_list_line_unchecked(row);
        assert_eq!(
            parsed.unwrap_err().kind(),
            io::ErrorKind::InvalidData,
            "{row}"
        );
    }
}
