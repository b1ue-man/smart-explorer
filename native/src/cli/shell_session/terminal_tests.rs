//! Terminal sessions through the same engine, attach service and clients as
//! remote ones, with a local shell in a pseudo-terminal (Unix) as the
//! transport: bytes in both directions, the size of the last active
//! participant, several terminals in one shell, notices, the end, the attach
//! protocol versions and the limits.
use super::terminal_engine::State;
use super::types::{Role, TerminalSize};

#[test]
fn terminal_task_the_last_active_participant_sets_the_size() {
    let size = |cols, rows| TerminalSize { cols, rows };
    let (sender, _receiver) = crossbeam_channel::bounded(4);
    let mut state = State::new(size(80, 24));
    let owner = state.add(Role::Owner, "owner".into(), sender.clone());
    let guest = state.add(Role::Attached, "guest".into(), sender);
    // Nobody reported a size yet: there is nothing to follow.
    assert_eq!(state.follow_active(), None);
    assert!(state.activity(owner, Some(size(80, 24))));
    assert_eq!(state.follow_active(), None);
    assert!(state.activity(guest, Some(size(100, 30))));
    assert_eq!(state.follow_active(), Some(size(100, 30)));
    assert_eq!(state.applied, size(100, 30));
    // Only a change is passed on.
    assert_eq!(state.follow_active(), None);
    assert!(state.activity(owner, None));
    assert_eq!(state.follow_active(), Some(size(80, 24)));
    // The active participant leaves: the other one's size applies.
    state.participants.remove(&owner);
    assert_eq!(state.follow_active(), Some(size(100, 30)));
    assert!(!state.activity(owner, None));
}

#[cfg(unix)]
mod pty {
    use crate::cli::shell_session::dialect;
    use crate::cli::shell_session::host::{host, host_terminal, OpenOptions};
    use crate::cli::shell_session::local_transport::LocalTransport;
    use crate::cli::shell_session::pty_test_transport::PtyTransport;
    use crate::cli::shell_session::registry::{self, Entry};
    use crate::cli::shell_session::server::TERMINAL_NEEDS_NEWER_CLIENT;
    use crate::cli::shell_session::terminal_client::TerminalClient;
    use crate::cli::shell_session::transports::Transport;
    use crate::cli::shell_session::types::{
        Dialect, SessionInfo, TerminalEvent, TerminalInfo, TerminalOptions, TerminalSize,
    };
    use crate::cli::shell_session::wire;
    use crate::cli::shell_session::SessionClient;
    use crossbeam_channel::Receiver;
    use std::collections::BTreeMap;
    use std::io::ErrorKind;
    use std::net::{Ipv4Addr, TcpStream};
    use std::path::Path;
    use std::time::{Duration, Instant};

    const SIZE: TerminalSize = TerminalSize { cols: 80, rows: 24 };
    const WAIT: Duration = Duration::from_secs(30);

    fn terminal_options(name: &str, attachable: bool) -> TerminalOptions {
        TerminalOptions {
            name: name.into(),
            owner: "test owner".into(),
            attachable,
            cwd: None,
            env: BTreeMap::new(),
            term: "xterm".into(),
            size: SIZE,
        }
    }

    /// A shell in a pseudo-terminal as a terminal session.
    fn open(name: &str, attachable: bool, registry_dir: &Path) -> TerminalClient {
        let transport = PtyTransport::spawn("/bin/sh", &["-i"], SIZE).unwrap();
        let options = terminal_options(name, attachable);
        host_terminal(Box::new(transport), None, "local", options, registry_dir).unwrap()
    }

    fn open_commands(name: &str, registry_dir: &Path) -> SessionClient {
        let start = dialect::start_line(Dialect::Posix);
        let options = OpenOptions {
            name: name.into(),
            owner: "test owner".into(),
            attachable: true,
            cwd: None,
            env: BTreeMap::new(),
        };
        let shell = LocalTransport::spawn("sh", &["-c", start]);
        host(
            Box::new(shell),
            Dialect::Posix,
            "local",
            options,
            registry_dir,
        )
        .unwrap()
    }

    fn attach(registry_dir: &Path, name: &str, label: &str, size: TerminalSize) -> TerminalClient {
        let entry = registry::find_in(registry_dir, name).unwrap();
        TerminalClient::attach(&entry, label, size).unwrap()
    }

    /// What one terminal client has received.
    struct Screen {
        events: Receiver<TerminalEvent>,
        text: Vec<u8>,
        notices: Vec<String>,
        ended: Option<(Option<i32>, String)>,
    }

    impl Screen {
        fn of(client: &TerminalClient) -> Self {
            Self {
                events: client.events(),
                text: Vec::new(),
                notices: Vec::new(),
                ended: None,
            }
        }

        fn take(&mut self, wait: Duration) {
            match self.events.recv_timeout(wait) {
                Ok(TerminalEvent::Output(bytes)) => self.text.extend_from_slice(&bytes),
                Ok(TerminalEvent::Notice(text)) => self.notices.push(text),
                Ok(TerminalEvent::Ended { exit_code, message }) => {
                    self.ended = Some((exit_code, message));
                }
                Err(_) => {}
            }
        }

        fn shows(&self, needle: &str) -> bool {
            let needle = needle.as_bytes();
            self.text.windows(needle.len()).any(|part| part == needle)
        }

        fn wait_for(&mut self, needle: &str) {
            let deadline = Instant::now() + WAIT;
            while !self.shows(needle) {
                let left = deadline.saturating_duration_since(Instant::now());
                assert!(
                    !left.is_zero(),
                    "{needle:?} never appeared on {:?}",
                    String::from_utf8_lossy(&self.text)
                );
                self.take(left);
            }
        }

        fn wait_notice(&mut self, text: &str) {
            let deadline = Instant::now() + WAIT;
            while !self.notices.iter().any(|notice| notice == text) {
                let left = deadline.saturating_duration_since(Instant::now());
                assert!(!left.is_zero(), "no notice {text:?} in {:?}", self.notices);
                self.take(left);
            }
        }

        fn wait_ended(&mut self) -> (Option<i32>, String) {
            let deadline = Instant::now() + WAIT;
            while self.ended.is_none() {
                let left = deadline.saturating_duration_since(Instant::now());
                assert!(!left.is_zero(), "the session did not end");
                self.take(left);
            }
            self.ended.clone().unwrap()
        }
    }

    /// A command that prints `word` although the typed line, which the
    /// terminal echoes, does not contain it.
    fn say(word: &str) -> String {
        let (head, tail) = word.split_at(word.len() / 2);
        format!("echo \"{head}\"\"{tail}\"\n")
    }

    fn wait_unlisted(registry_dir: &Path) {
        let deadline = Instant::now() + Duration::from_secs(10);
        while !registry::list_in(registry_dir).is_empty() {
            assert!(Instant::now() < deadline, "the session entry stayed listed");
            std::thread::sleep(Duration::from_millis(50));
        }
    }

    #[test]
    fn terminal_task_bytes_pass_unchanged_in_both_directions() {
        let dir = tempfile::tempdir().unwrap();
        let owner = open("bytes", false, dir.path());
        let mut screen = Screen::of(&owner);
        // Remote to local: the escape sequence arrives as the shell printed it.
        owner.input(b"printf 'a\\033[31mb\\033[0m\\n'\n").unwrap();
        screen.wait_for("a\x1b[31mb\x1b[0m");
        // Local to remote: in raw mode every byte arrives as it was typed.
        owner
            .input(b"stty raw -echo; echo \"RA\"\"W\"; od -An -tx1 -N6\n")
            .unwrap();
        screen.wait_for("RAW");
        owner.input(&[0x00, 0x01, 0x1b, 0x7f, 0xc3, 0xa9]).unwrap();
        screen.wait_for("00 01 1b 7f c3 a9");
        owner.leave();
        screen.wait_ended();
    }

    #[test]
    fn terminal_task_large_output_arrives_complete_and_in_order() {
        let dir = tempfile::tempdir().unwrap();
        let owner = open("flow", false, dir.path());
        let mut screen = Screen::of(&owner);
        owner.input(b"seq 1 30000; echo \"EN\"\"D\"\n").unwrap();
        screen.wait_for("END");
        let expected: String = (1..=30000).map(|number| format!("{number}\r\n")).collect();
        assert!(
            screen.shows(&expected),
            "the output lost or reordered bytes"
        );
        owner.leave();
    }

    #[test]
    fn terminal_task_the_remote_terminal_has_the_size_of_the_last_active_terminal() {
        let dir = tempfile::tempdir().unwrap();
        let owner = open("size", true, dir.path());
        let mut owner_screen = Screen::of(&owner);
        owner.input(b"echo \"S$(stty size)E\"\n").unwrap();
        owner_screen.wait_for("S24 80E");
        let guest = attach(
            dir.path(),
            "size",
            "guest",
            TerminalSize {
                cols: 100,
                rows: 30,
            },
        );
        let mut guest_screen = Screen::of(&guest);
        guest.input(b"echo \"T$(stty size)E\"\n").unwrap();
        guest_screen.wait_for("T30 100E");
        owner_screen.wait_for("T30 100E");
        owner.input(b"echo \"U$(stty size)E\"\n").unwrap();
        owner_screen.wait_for("U24 80E");
        // A size report makes its sender the active terminal, too.
        guest
            .resize(TerminalSize {
                cols: 120,
                rows: 40,
            })
            .unwrap();
        guest.input(b"echo \"V$(stty size)E\"\n").unwrap();
        guest_screen.wait_for("V40 120E");
        assert!(guest.resize(TerminalSize { cols: 0, rows: 5 }).is_err());
        owner.leave();
    }

    #[test]
    fn terminal_task_a_second_terminal_sees_the_output_and_types_into_the_same_shell() {
        let dir = tempfile::tempdir().unwrap();
        let owner = open("shared", true, dir.path());
        let mut owner_screen = Screen::of(&owner);
        let guest = attach(dir.path(), "shared", "guest", SIZE);
        let mut guest_screen = Screen::of(&guest);
        owner.input(say("FROMOWNER").as_bytes()).unwrap();
        guest_screen.wait_for("FROMOWNER");
        owner_screen.wait_for("FROMOWNER");
        guest.input(say("FROMGUEST").as_bytes()).unwrap();
        owner_screen.wait_for("FROMGUEST");
        guest_screen.wait_for("FROMGUEST");
        guest
            .input(b"export SHARED=yes; echo \"DO\"\"NE\"\n")
            .unwrap();
        guest_screen.wait_for("DONE");
        owner.input(b"echo \"SH-$SHARED-ED\"\n").unwrap();
        owner_screen.wait_for("SH-yes-ED");
        owner_screen.wait_notice("guest attached");
        guest.leave();
        owner_screen.wait_notice("guest detached");
        owner.leave();
    }

    #[test]
    fn terminal_task_a_terminals_input_reaches_the_others_only_as_the_remote_echo() {
        let dir = tempfile::tempdir().unwrap();
        let owner = open("quiet", true, dir.path());
        let mut owner_screen = Screen::of(&owner);
        let guest = attach(dir.path(), "quiet", "guest", SIZE);
        let mut guest_screen = Screen::of(&guest);
        // The remote does not echo: the guest's password stays with the guest.
        owner
            .input(b"stty -echo; echo \"RE\"\"ADY\"; read -r PW; echo \"L${#PW}E\"\n")
            .unwrap();
        guest_screen.wait_for("READY");
        guest.input(b"secret-xyz\n").unwrap();
        owner_screen.wait_for("L10E");
        guest_screen.wait_for("L10E");
        assert!(!owner_screen.shows("secret-xyz"));
        assert!(!guest_screen.shows("secret-xyz"));
        owner.leave();
    }

    #[test]
    fn terminal_task_notices_carry_no_control_characters() {
        let dir = tempfile::tempdir().unwrap();
        let owner = open("notices", true, dir.path());
        let mut owner_screen = Screen::of(&owner);
        let guest = attach(dir.path(), "notices", "evil\x1b[2J\x07", SIZE);
        owner_screen.wait_notice("evil[2J attached");
        guest.leave();
        owner_screen.wait_notice("evil[2J detached");
        let controls = |text: &String| text.chars().any(char::is_control);
        assert!(!owner_screen.notices.iter().any(controls));
        owner.leave();
        let (_, message) = owner_screen.wait_ended();
        assert!(!message.chars().any(char::is_control));
    }

    #[test]
    fn terminal_task_the_end_of_the_shell_reaches_everyone_and_unlists_the_session() {
        let dir = tempfile::tempdir().unwrap();
        let owner = open("exit", true, dir.path());
        let mut owner_screen = Screen::of(&owner);
        let guest = attach(dir.path(), "exit", "guest", SIZE);
        let mut guest_screen = Screen::of(&guest);
        guest.input(b"exit 3\n").unwrap();
        for screen in [&mut owner_screen, &mut guest_screen] {
            let (code, message) = screen.wait_ended();
            assert_eq!(code, Some(3));
            assert!(message.contains("status 3"), "{message}");
        }
        wait_unlisted(dir.path());
        assert!(owner.wait_ended(Duration::from_secs(5)));
        assert_eq!(owner.ended().map(|(code, _)| code), Some(Some(3)));
    }

    #[test]
    fn terminal_task_the_owner_leaving_or_a_cancel_ends_the_session_for_everyone() {
        let dir = tempfile::tempdir().unwrap();
        let owner = open("leave", true, dir.path());
        let guest = attach(dir.path(), "leave", "guest", SIZE);
        let mut guest_screen = Screen::of(&guest);
        let mut owner_screen = Screen::of(&owner);
        owner.leave();
        let (_, message) = guest_screen.wait_ended();
        assert!(message.contains("owner"), "{message}");
        owner_screen.wait_ended();
        wait_unlisted(dir.path());
        // A cancel by an attached terminal does the same and names it.
        let other = open("cancel", true, dir.path());
        let mut other_screen = Screen::of(&other);
        let canceller = attach(dir.path(), "cancel", "canceller", SIZE);
        canceller.cancel();
        let (code, message) = other_screen.wait_ended();
        assert_eq!(code, None);
        assert!(message.contains("canceller"), "{message}");
        wait_unlisted(dir.path());
    }

    /// The first answer of the attach service to a hello of `version`.
    fn hello(entry: &Entry, version: u32, token: &str) -> (TcpStream, u8, Vec<u8>) {
        let mut stream = TcpStream::connect((Ipv4Addr::LOCALHOST, entry.port)).unwrap();
        stream.set_read_timeout(Some(WAIT)).unwrap();
        let hello = wire::Hello {
            version,
            session: entry.id.clone(),
            token: token.into(),
            label: "raw".into(),
        };
        wire::write_json(&mut stream, wire::HELLO, &hello).unwrap();
        let (tag, payload) = wire::read_frame(&mut stream).unwrap();
        (stream, tag, payload)
    }

    fn denied(payload: &[u8]) -> String {
        wire::decode::<wire::Text>(payload).unwrap().text
    }

    #[test]
    fn terminal_task_the_hello_version_decides_which_sessions_a_client_may_join() {
        let dir = tempfile::tempdir().unwrap();
        let _terminal = open("v-terminal", true, dir.path());
        let _commands = open_commands("v-commands", dir.path());
        let terminal = registry::find_in(dir.path(), "v-terminal").unwrap();
        let commands = registry::find_in(dir.path(), "v-commands").unwrap();
        // Version 1 only reaches command sessions; version 2 reaches both.
        let (_, tag, payload) = hello(&terminal, 1, &terminal.token);
        assert_eq!(tag, wire::DENIED);
        assert_eq!(denied(&payload), TERMINAL_NEEDS_NEWER_CLIENT);
        let (_, tag, payload) = hello(&terminal, 2, &terminal.token);
        assert_eq!(tag, wire::WELCOME);
        let info: TerminalInfo = wire::decode(&payload).unwrap();
        assert_eq!(info.name, "v-terminal");
        for version in [1, 2] {
            let (_, tag, payload) = hello(&commands, version, &commands.token);
            assert_eq!(tag, wire::WELCOME, "version {version}");
            let info: SessionInfo = wire::decode(&payload).unwrap();
            assert_eq!(info.name, "v-commands");
        }
        // Other versions, and a wrong token (which must not tell the kind).
        for entry in [&terminal, &commands] {
            let (_, tag, payload) = hello(entry, 3, &entry.token);
            assert_eq!(
                (tag, denied(&payload).as_str()),
                (wire::DENIED, "unsupported attach protocol version")
            );
        }
        let (_, tag, payload) = hello(&terminal, 1, &"00".repeat(32));
        assert_eq!(
            (tag, denied(&payload).as_str()),
            (wire::DENIED, "permission denied")
        );
        // Each client type refuses the other kind of session by itself.
        let error = SessionClient::attach(&terminal, "old style").err().unwrap();
        assert!(error.contains("is a terminal session"), "{error}");
        let error = TerminalClient::attach(&commands, "terminal", SIZE)
            .err()
            .unwrap();
        assert!(error.contains("is not a terminal session"), "{error}");
    }

    #[test]
    fn terminal_task_frames_of_the_other_kind_of_session_end_the_connection() {
        let dir = tempfile::tempdir().unwrap();
        let _terminal = open("f-terminal", true, dir.path());
        let _commands = open_commands("f-commands", dir.path());
        let terminal = registry::find_in(dir.path(), "f-terminal").unwrap();
        let commands = registry::find_in(dir.path(), "f-commands").unwrap();
        let cases = [
            (&commands, 1, wire::TERM_INPUT, b"x".to_vec()),
            (
                &commands,
                2,
                wire::TERM_RESIZE,
                br#"{"cols":80,"rows":24}"#.to_vec(),
            ),
            (&terminal, 2, wire::SUBMIT, b"{}".to_vec()),
            (&terminal, 2, wire::INPUT, b"x".to_vec()),
        ];
        for (entry, version, tag, payload) in cases {
            let (mut stream, answer, _) = hello(entry, version, &entry.token);
            assert_eq!(answer, wire::WELCOME);
            wire::write_frame(&mut stream, tag, &payload).unwrap();
            // The service closes the connection: reading ends, no timeout.
            let ended = loop {
                match wire::read_frame(&mut stream) {
                    Ok(_) => {}
                    Err(error) => break error,
                }
            };
            let waited = matches!(ended.kind(), ErrorKind::WouldBlock | ErrorKind::TimedOut);
            assert!(!waited, "tag {tag} kept the connection open");
        }
    }

    #[test]
    fn terminal_task_a_ninth_attached_terminal_is_refused() {
        let dir = tempfile::tempdir().unwrap();
        let owner = open("limit", true, dir.path());
        let entry = registry::find_in(dir.path(), "limit").unwrap();
        let guests: Vec<TerminalClient> = (0..8)
            .map(|index| {
                let label = format!("guest{index}");
                TerminalClient::attach(&entry, &label, SIZE).unwrap()
            })
            .collect();
        let error = TerminalClient::attach(&entry, "ninth", SIZE).err().unwrap();
        assert!(error.contains("8 clients are already attached"), "{error}");
        drop(guests);
        owner.leave();
    }

    #[test]
    fn terminal_task_opening_checks_name_term_and_size_before_anything_runs() {
        let dir = tempfile::tempdir().unwrap();
        let bad_term = |term: &str| TerminalOptions {
            term: term.into(),
            ..terminal_options("checks", false)
        };
        let bad_size = TerminalOptions {
            size: TerminalSize { cols: 0, rows: 24 },
            ..terminal_options("checks", false)
        };
        let bad_name = terminal_options("not a name", false);
        for options in [
            bad_term("x y"),
            bad_term(""),
            bad_term(&"x".repeat(65)),
            bad_size,
            bad_name,
        ] {
            let transport = PtyTransport::spawn("/bin/sh", &["-i"], SIZE).unwrap();
            let shell = transport.input();
            let refused = host_terminal(Box::new(transport), None, "local", options, dir.path());
            assert!(refused.is_err());
            shell.cancel();
        }
        assert!(registry::list_in(dir.path()).is_empty());
    }
}
