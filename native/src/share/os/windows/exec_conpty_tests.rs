//! ConPTY launcher tests; they run only on Windows 10 1809 / Server 2019 or newer.
use std::io::{Read, Write};
use std::sync::mpsc;
use std::time::Instant;

use super::*;

const LIMIT: Duration = Duration::from_secs(60);

fn size(cols: u16, rows: u16) -> ExecTerminalSize {
    ExecTerminalSize { cols, rows }
}

fn test_environment() -> BTreeMap<String, String> {
    std::env::vars_os()
        .filter_map(|(name, value)| Some((name.into_string().ok()?, value.into_string().ok()?)))
        .filter(|(name, _)| !name.is_empty() && !name.contains('='))
        .collect()
}

/// Output of a pseudoconsole, collected by a reader thread.
struct Screen {
    chunks: mpsc::Receiver<Vec<u8>>,
    bytes: Vec<u8>,
    ended: bool,
}

impl Screen {
    fn new(mut output: File) -> Self {
        let (sender, chunks) = mpsc::channel();
        std::thread::spawn(move || {
            let mut buffer = [0u8; 4096];
            loop {
                match output.read(&mut buffer) {
                    Ok(0) | Err(_) => break,
                    Ok(read) => {
                        if sender.send(buffer[..read].to_vec()).is_err() {
                            break;
                        }
                    }
                }
            }
        });
        Self {
            chunks,
            bytes: Vec::new(),
            ended: false,
        }
    }

    fn text(&self) -> String {
        String::from_utf8_lossy(&self.bytes).into_owned()
    }

    /// Collects output until `done` accepts it, the pipe broke or `limit`
    /// elapsed. True when `done` accepted the output.
    fn read_until(&mut self, limit: Duration, done: impl Fn(&str) -> bool) -> bool {
        let deadline = Instant::now() + limit;
        loop {
            if done(&self.text()) {
                return true;
            }
            if self.ended {
                return false;
            }
            let left = deadline.saturating_duration_since(Instant::now());
            if left.is_zero() {
                return false;
            }
            match self.chunks.recv_timeout(left) {
                Ok(chunk) => self.bytes.extend(chunk),
                Err(mpsc::RecvTimeoutError::Timeout) => return false,
                Err(mpsc::RecvTimeoutError::Disconnected) => self.ended = true,
            }
        }
    }

    /// Reads until the output pipe breaks; true when it did within `limit`.
    fn read_to_end(&mut self, limit: Duration) -> bool {
        self.read_until(limit, |_| false);
        self.ended
    }
}

fn exit_code(conpty: &Conpty, limit: Duration) -> u32 {
    conpty
        .wait(limit)
        .expect("waiting for the process failed")
        .unwrap_or_else(|| panic!("the process did not exit within {limit:?}"))
}

/// Types a command that prints the window width as `<tag>=<width>;` and waits
/// for that text. The echo of the command itself never contains it.
fn query_width(screen: &mut Screen, input: &mut File, tag: &str, expected: u16) -> bool {
    let command = format!("\"{tag}=$($Host.UI.RawUI.WindowSize.Width);\"\r");
    input
        .write_all(command.as_bytes())
        .expect("writing to the pseudoconsole input failed");
    let wanted = format!("{tag}={expected};");
    screen.read_until(Duration::from_secs(10), |text| text.contains(&wanted))
}

#[test]
fn terminal_task_conpty_is_available_on_supported_windows() {
    assert!(
        terminal_supported(),
        "CreatePseudoConsole, ResizePseudoConsole and ClosePseudoConsole must exist in kernel32.dll"
    );
}

#[test]
fn terminal_task_cmd_runs_in_a_pseudoconsole_with_vt_output() {
    let program = Program::system("cmd.exe", "cmd.exe /D /C echo terminal-ok").unwrap();
    let (mut conpty, output, _input) =
        start(&program, &test_environment(), None, size(100, 30)).unwrap();
    let mut screen = Screen::new(output);
    assert_eq!(exit_code(&conpty, LIMIT), 0);
    let _closer = conpty.close_console();
    assert!(
        screen.read_to_end(LIMIT),
        "the output pipe did not break after the pseudoconsole closed"
    );
    let text = screen.text();
    assert!(text.contains("terminal-ok"), "output was {text:?}");
    assert!(
        text.contains("\u{1b}["),
        "no CSI sequence in the output {text:?}"
    );
}

#[test]
fn terminal_task_powershell_follows_resize_and_returns_its_exit_code() {
    let program = Program::login_shell().unwrap();
    let (mut conpty, output, mut input) =
        start(&program, &test_environment(), None, size(100, 30)).unwrap();
    let mut screen = Screen::new(output);
    assert!(
        screen.read_until(LIMIT, |text| text.contains("PS ") && text.contains("> ")),
        "no PowerShell prompt, output was {:?}",
        screen.text()
    );
    assert!(
        query_width(&mut screen, &mut input, "w1", 100),
        "initial width missing, output was {:?}",
        screen.text()
    );

    conpty.resize(size(120, 40)).unwrap();
    // The pseudoconsole applies a resize asynchronously; the first command
    // typed right after it may still see the old width.
    let resized = ["a", "b", "c", "d", "e"]
        .iter()
        .any(|attempt| query_width(&mut screen, &mut input, &format!("w2{attempt}"), 120));
    assert!(resized, "width 120 missing, output was {:?}", screen.text());

    input.write_all(b"exit 7\r").unwrap();
    assert_eq!(exit_code(&conpty, LIMIT), 7);
    let _closer = conpty.close_console();
    assert!(screen.read_to_end(LIMIT), "the output pipe did not break");
}

#[test]
fn terminal_task_closing_the_console_ends_a_running_command() {
    let program = Program::system("cmd.exe", "cmd.exe /D /C ping -t 127.0.0.1").unwrap();
    let (mut conpty, output, _input) =
        start(&program, &test_environment(), None, size(100, 30)).unwrap();
    let mut screen = Screen::new(output);
    assert!(
        screen.read_until(LIMIT, |text| text.contains("127.0.0.1")),
        "ping printed nothing, output was {:?}",
        screen.text()
    );
    let limit = Duration::from_secs(15);
    let closed_at = Instant::now();
    let _closer = conpty.close_console();
    assert!(
        screen.read_to_end(limit),
        "the output pipe did not break within {limit:?}"
    );
    exit_code(&conpty, limit.saturating_sub(closed_at.elapsed()));
    assert!(closed_at.elapsed() < limit);
}

#[test]
fn terminal_task_control_c_ends_a_command_in_the_pseudoconsole() {
    // Without Ctrl+C ping runs for about a minute (60 echo requests, one per
    // second); the byte 3 on the pseudoconsole input must end it at once, as
    // Ctrl+C does over SSH. `start` clears an inherited "ignore Ctrl+C".
    let program = Program::system("ping.exe", "ping.exe -n 60 127.0.0.1").unwrap();
    let (mut conpty, output, mut input) =
        start(&program, &test_environment(), None, size(100, 30)).unwrap();
    let mut screen = Screen::new(output);
    assert!(
        screen.read_until(LIMIT, |text| text.contains("127.0.0.1")),
        "ping printed nothing, output was {:?}",
        screen.text()
    );
    // ping is attached to the console once it printed; give it a moment to
    // install its own Ctrl+C handler, then type Ctrl+C.
    std::thread::sleep(Duration::from_secs(1));
    input.write_all(b"\x03").unwrap();
    let limit = Duration::from_secs(10);
    let ended = conpty.wait(limit).unwrap();
    let _closer = conpty.close_console();
    assert!(
        ended.is_some(),
        "Ctrl+C did not end ping within {limit:?}, output was {:?}",
        screen.text()
    );
    assert!(screen.read_to_end(LIMIT), "the output pipe did not break");
}

#[test]
fn terminal_task_resize_after_close_reports_a_closed_console() {
    let program = Program::system("cmd.exe", "cmd.exe /D /C exit 0").unwrap();
    let (mut conpty, output, _input) =
        start(&program, &test_environment(), None, size(80, 24)).unwrap();
    let mut screen = Screen::new(output);
    assert_eq!(exit_code(&conpty, LIMIT), 0);
    let _closer = conpty.close_console();
    assert!(screen.read_to_end(LIMIT));
    assert!(conpty.close_console().is_none());
    let error = conpty.resize(size(100, 30)).unwrap_err();
    assert_eq!(error.kind(), io::ErrorKind::BrokenPipe);
}

#[test]
fn terminal_task_coordinates_reject_unusable_sizes() {
    for (cols, rows) in [(0, 24), (80, 0), (32768, 24), (80, 32768)] {
        let error = to_coord(size(cols, rows)).err().expect("unusable size");
        assert_eq!(error.kind(), io::ErrorKind::InvalidInput, "{cols}x{rows}");
    }
    let largest = to_coord(size(32767, 1)).unwrap();
    assert_eq!((largest.X, largest.Y), (32767, 1));
}

#[test]
fn terminal_task_environment_block_is_sorted_without_regard_to_case() {
    let environment = BTreeMap::from([
        ("b".to_string(), "2".to_string()),
        ("A".to_string(), "1".to_string()),
        ("c".to_string(), "x y".to_string()),
    ]);
    let block = environment_block(&environment);
    assert_eq!(String::from_utf16(&block).unwrap(), "A=1\0b=2\0c=x y\0\0");
}

#[test]
fn terminal_task_environment_block_keeps_one_entry_per_name_ignoring_case() {
    let environment = BTreeMap::from([
        ("PATH".to_string(), "q".to_string()),
        ("Path".to_string(), "p".to_string()),
    ]);
    let block = environment_block(&environment);
    assert_eq!(String::from_utf16(&block).unwrap(), "Path=p\0\0");
}

#[test]
fn terminal_task_empty_environment_block_is_two_nul_characters() {
    assert_eq!(environment_block(&BTreeMap::new()), vec![0, 0]);
}

#[test]
fn terminal_task_login_shell_is_the_absolute_system_powershell() {
    let program = Program::login_shell().unwrap();
    assert_eq!(program.application.last(), Some(&0));
    let application = String::from_utf16(&program.application[..program.application.len() - 1])
        .expect("application path is valid UTF-16");
    let path = std::path::Path::new(&application);
    assert!(path.is_absolute(), "{application}");
    assert!(path.is_file(), "{application}");
    assert!(
        application
            .to_ascii_lowercase()
            .ends_with(r"\windowspowershell\v1.0\powershell.exe"),
        "{application}"
    );
    assert_eq!(program.command_line.last(), Some(&0));
    let command_line = String::from_utf16(&program.command_line[..program.command_line.len() - 1])
        .expect("command line is valid UTF-16");
    assert_eq!(command_line, "powershell.exe -NoLogo");
}
