//! The local console of a `se shell` terminal session on Windows, after the
//! Win32-OpenSSH client (`contrib/win32/win32compat/tncon.c`): VT input without
//! line editing, echo and processed Ctrl+C (the key is the byte 3 for the
//! remote program), VT output with the UTF-8 code page, keys and window size
//! changes read as console input records, and the remote output decoded as
//! UTF-8 across chunks because Rust's standard output refuses invalid UTF-8 on a
//! console. The saved modes and code page come back when the terminal is
//! released and at once in the control handler (Ctrl+Break, closing the
//! window, logoff, shutdown).
use super::LocalEvent;
use crossbeam_channel::Sender;
use std::io::{self, Write};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::{Duration, Instant};
use windows_sys::Win32::Foundation::{
    BOOL, FALSE, HANDLE, INVALID_HANDLE_VALUE, TRUE, WAIT_OBJECT_0, WAIT_TIMEOUT,
};
use windows_sys::Win32::System::Console::{
    GetConsoleMode, GetConsoleOutputCP, GetConsoleScreenBufferInfo, GetStdHandle,
    ReadConsoleInputW, SetConsoleCtrlHandler, SetConsoleMode, SetConsoleOutputCP, CONSOLE_MODE,
    CONSOLE_SCREEN_BUFFER_INFO, CTRL_BREAK_EVENT, CTRL_CLOSE_EVENT, CTRL_C_EVENT,
    CTRL_LOGOFF_EVENT, CTRL_SHUTDOWN_EVENT, DISABLE_NEWLINE_AUTO_RETURN, ENABLE_ECHO_INPUT,
    ENABLE_LINE_INPUT, ENABLE_PROCESSED_INPUT, ENABLE_PROCESSED_OUTPUT,
    ENABLE_VIRTUAL_TERMINAL_INPUT, ENABLE_VIRTUAL_TERMINAL_PROCESSING, ENABLE_WINDOW_INPUT,
    INPUT_RECORD, KEY_EVENT, STD_INPUT_HANDLE, STD_OUTPUT_HANDLE, WINDOW_BUFFER_SIZE_EVENT,
};
use windows_sys::Win32::System::Threading::WaitForSingleObject;

const UTF8_CODE_PAGE: u32 = 65001;
/// How long the input reader waits before it looks at the stop flag.
const STOP_CHECK_MS: u32 = 200;
/// Input records read at once.
const RECORD_COUNT: u32 = 128;
/// The Alt key; Alt+numpad entry delivers its character when Alt is released.
const VK_MENU: u16 = 0x12;
/// Exit code after a console control event.
const EXIT_TERMINATED: i32 = 130;
/// How long the control handler gives the session to leave before Windows ends
/// the process (closing the window allows about five seconds).
const CLOSE_GRACE: Duration = Duration::from_secs(3);
const REPLACEMENT: char = '\u{FFFD}';

/// What to put back: the standard handles (as numbers, to pass between
/// threads), their modes and the output code page.
#[derive(Clone, Copy)]
struct Restore {
    input: isize,
    output: isize,
    input_mode: CONSOLE_MODE,
    output_mode: CONSOLE_MODE,
    code_page: u32,
}

static RESTORE: Mutex<Option<Restore>> = Mutex::new(None);
static EVENTS: Mutex<Option<Sender<LocalEvent>>> = Mutex::new(None);
/// Set when the terminal is released; the control handler waits for it.
static FINISHED: AtomicBool = AtomicBool::new(false);

/// A console in virtual-terminal mode that reports what happens to it.
/// Dropping it restores the modes and the code page.
pub(in crate::cli) struct LocalTerminal {
    stop: Arc<AtomicBool>,
}

impl LocalTerminal {
    /// Switches the console to virtual-terminal mode, registers the control
    /// handler and starts the thread that sends `events`.
    pub(in crate::cli) fn enter(events: Sender<LocalEvent>) -> Result<Self, String> {
        let saved = apply_modes()?;
        *locked(&RESTORE) = Some(saved);
        *locked(&EVENTS) = Some(events.clone());
        FINISHED.store(false, Ordering::SeqCst);
        let terminal = Self {
            stop: Arc::new(AtomicBool::new(false)),
        };
        // SAFETY: `on_control` has the handler signature and lives for the
        // whole process.
        if unsafe { SetConsoleCtrlHandler(Some(on_control), TRUE) } == 0 {
            return Err(os_error("console control handler"));
        }
        let stop = terminal.stop.clone();
        let (input, output) = (saved.input, saved.output);
        std::thread::Builder::new()
            .name("se-term-input".into())
            .spawn(move || read_input(input, output, events, stop))
            .map_err(|error| format!("input reader: {error}"))?;
        Ok(terminal)
    }
}

impl Drop for LocalTerminal {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
        restore_now();
        // SAFETY: removes the routine `enter` added (a no-op if it was not).
        unsafe { SetConsoleCtrlHandler(Some(on_control), FALSE) };
        *locked(&EVENTS) = None;
        FINISHED.store(true, Ordering::SeqCst);
    }
}

/// Whether the console accepts the session's modes; they are restored at once.
pub(super) fn check_raw_mode() -> Result<(), String> {
    let saved = apply_modes()?;
    restore_console(&saved);
    Ok(())
}

/// Columns and rows of the visible console window.
pub(super) fn window_size() -> Option<(u16, u16)> {
    let (_, output) = console_handles().ok()?;
    window_size_of(output)
}

/// Writes the remote terminal's bytes to the console as valid UTF-8.
#[derive(Default)]
pub(in crate::cli) struct TerminalOutput {
    decoder: Utf8Decoder,
}

impl TerminalOutput {
    pub(in crate::cli) fn write(&mut self, bytes: &[u8]) -> io::Result<()> {
        let text = self.decoder.push(bytes);
        if text.is_empty() {
            return Ok(());
        }
        let mut stdout = io::stdout().lock();
        stdout.write_all(text.as_bytes())?;
        stdout.flush()
    }
}

/// `what` and the last operating system error as one message.
fn os_error(what: &str) -> String {
    format!("{what}: {}", io::Error::last_os_error())
}

fn locked<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

fn console_handles() -> Result<(HANDLE, HANDLE), String> {
    // SAFETY: GetStdHandle only reads this process's standard handle table.
    let (input, output) = unsafe {
        (
            GetStdHandle(STD_INPUT_HANDLE),
            GetStdHandle(STD_OUTPUT_HANDLE),
        )
    };
    for handle in [input, output] {
        if handle.is_null() || handle == INVALID_HANDLE_VALUE {
            return Err("no console is attached".to_string());
        }
    }
    Ok((input, output))
}

/// Input mode of the session: VT sequences and window records, no line input,
/// echo or processed Ctrl+C.
fn raw_input_mode(saved: CONSOLE_MODE) -> CONSOLE_MODE {
    (saved | ENABLE_VIRTUAL_TERMINAL_INPUT | ENABLE_WINDOW_INPUT)
        & !(ENABLE_PROCESSED_INPUT | ENABLE_LINE_INPUT | ENABLE_ECHO_INPUT)
}

/// Output mode of the session: VT processing (`hold_wrap` also delays the wrap
/// at the last column, as terminals do; very old Windows 10 builds lack it).
fn vt_output_mode(saved: CONSOLE_MODE, hold_wrap: bool) -> CONSOLE_MODE {
    let mode = saved | ENABLE_PROCESSED_OUTPUT | ENABLE_VIRTUAL_TERMINAL_PROCESSING;
    if hold_wrap {
        mode | DISABLE_NEWLINE_AUTO_RETURN
    } else {
        mode
    }
}

/// Sets the session's modes and code page; returns what to restore.
fn apply_modes() -> Result<Restore, String> {
    let (input, output) = console_handles()?;
    let (mut input_mode, mut output_mode): (CONSOLE_MODE, CONSOLE_MODE) = (0, 0);
    // SAFETY: both are console handles checked above and the modes are writable.
    if unsafe { GetConsoleMode(input, &mut input_mode) } == 0 {
        return Err(os_error("console input mode"));
    }
    // SAFETY: as above.
    if unsafe { GetConsoleMode(output, &mut output_mode) } == 0 {
        return Err(os_error("console output mode"));
    }
    let saved = Restore {
        input: input as isize,
        output: output as isize,
        input_mode,
        output_mode,
        // SAFETY: GetConsoleOutputCP takes no arguments.
        code_page: unsafe { GetConsoleOutputCP() },
    };
    // SAFETY (the three calls): the handles are valid console handles.
    let output_set = unsafe {
        SetConsoleMode(output, vt_output_mode(output_mode, true)) != 0
            || SetConsoleMode(output, vt_output_mode(output_mode, false)) != 0
    };
    if !output_set {
        return Err(os_error("console output mode"));
    }
    // SAFETY: as above.
    if unsafe { SetConsoleMode(input, raw_input_mode(input_mode)) } == 0 {
        let error = os_error("console input mode");
        restore_console(&saved);
        return Err(error);
    }
    // SAFETY: SetConsoleOutputCP only takes a code page number.
    if unsafe { SetConsoleOutputCP(UTF8_CODE_PAGE) } == 0 {
        let error = os_error("console code page");
        restore_console(&saved);
        return Err(error);
    }
    Ok(saved)
}

fn restore_console(saved: &Restore) {
    // SAFETY: puts back what `apply_modes` read from the same handles.
    unsafe {
        SetConsoleMode(saved.input as HANDLE, saved.input_mode);
        SetConsoleMode(saved.output as HANDLE, saved.output_mode);
        SetConsoleOutputCP(saved.code_page);
    }
}

/// Restores the saved console state once, from whichever thread comes first.
fn restore_now() {
    if let Some(saved) = locked(&RESTORE).take() {
        restore_console(&saved);
    }
}

/// Console control events: Ctrl+C is swallowed (the key is a byte); Ctrl+Break,
/// closing the window, logoff and shutdown restore the console, ask the session
/// to end and give it a moment to leave.
unsafe extern "system" fn on_control(kind: u32) -> BOOL {
    match kind {
        CTRL_C_EVENT => TRUE,
        CTRL_BREAK_EVENT | CTRL_CLOSE_EVENT | CTRL_LOGOFF_EVENT | CTRL_SHUTDOWN_EVENT => {
            restore_now();
            if let Some(events) = locked(&EVENTS).as_ref() {
                let _ = events.send(LocalEvent::Terminate(EXIT_TERMINATED));
            }
            if kind != CTRL_BREAK_EVENT {
                let deadline = Instant::now() + CLOSE_GRACE;
                while !FINISHED.load(Ordering::SeqCst) && Instant::now() < deadline {
                    std::thread::sleep(Duration::from_millis(25));
                }
            }
            TRUE
        }
        _ => FALSE,
    }
}

/// Columns and rows of the visible window of the screen buffer.
fn window_size_of(output: HANDLE) -> Option<(u16, u16)> {
    // SAFETY: an all-zero buffer info is valid; the call fills it in.
    let mut info: CONSOLE_SCREEN_BUFFER_INFO = unsafe { std::mem::zeroed() };
    // SAFETY: `output` is a console handle and `info` a writable structure.
    if unsafe { GetConsoleScreenBufferInfo(output, &mut info) } == 0 {
        return None;
    }
    let window = info.srWindow;
    let cols = u16::try_from(i32::from(window.Right) - i32::from(window.Left) + 1).ok()?;
    let rows = u16::try_from(i32::from(window.Bottom) - i32::from(window.Top) + 1).ok()?;
    (cols > 0 && rows > 0).then_some((cols, rows))
}

/// Reads console input records until the terminal is released.
fn read_input(input: isize, output: isize, events: Sender<LocalEvent>, stop: Arc<AtomicBool>) {
    let (input, output) = (input as HANDLE, output as HANDLE);
    // SAFETY: an all-zero INPUT_RECORD is valid plain data.
    let mut records: Vec<INPUT_RECORD> = vec![unsafe { std::mem::zeroed() }; RECORD_COUNT as usize];
    let mut keys = KeyTranslator::default();
    let mut last_size = window_size_of(output);
    while !stop.load(Ordering::SeqCst) {
        // SAFETY: `input` is the console input handle.
        let wait = unsafe { WaitForSingleObject(input, STOP_CHECK_MS) };
        if wait == WAIT_TIMEOUT {
            continue;
        }
        let mut read = 0u32;
        // SAFETY: `records` holds RECORD_COUNT records and `read` is writable.
        let ok = wait == WAIT_OBJECT_0
            && unsafe { ReadConsoleInputW(input, records.as_mut_ptr(), RECORD_COUNT, &mut read) }
                != 0;
        if !ok {
            let _ = events.send(LocalEvent::InputClosed);
            return;
        }
        let mut typed = Vec::new();
        let mut resized = false;
        for record in records.iter().take(read as usize) {
            match u32::from(record.EventType) {
                KEY_EVENT => {
                    // SAFETY: the event type says the union holds a key record.
                    let key = unsafe { record.Event.KeyEvent };
                    keys.push(
                        KeyInput {
                            key_down: key.bKeyDown != 0,
                            repeat: key.wRepeatCount,
                            virtual_key: key.wVirtualKeyCode,
                            // SAFETY: the character is always a UTF-16 unit.
                            unit: unsafe { key.uChar.UnicodeChar },
                        },
                        &mut typed,
                    );
                }
                WINDOW_BUFFER_SIZE_EVENT => resized = true,
                _ => {}
            }
        }
        if !typed.is_empty() && events.send(LocalEvent::Input(typed)).is_err() {
            return;
        }
        if resized {
            let size = window_size_of(output);
            if let (Some((cols, rows)), true) = (size, size != last_size) {
                last_size = size;
                if events.send(LocalEvent::Resized(cols, rows)).is_err() {
                    return;
                }
            }
        }
    }
}

/// One key record, reduced to what the translation needs.
#[derive(Clone, Copy, Debug)]
struct KeyInput {
    key_down: bool,
    repeat: u16,
    virtual_key: u16,
    /// The UTF-16 unit of the key, 0 if it has none.
    unit: u16,
}

/// Turns key records into UTF-8: key presses (and the release of Alt, which
/// carries an Alt+numpad character) with a character; surrogate pairs are
/// joined across records, lone surrogates become U+FFFD; a repeat count
/// repeats the character.
#[derive(Default)]
struct KeyTranslator {
    high: Option<u16>,
}

impl KeyTranslator {
    fn push(&mut self, key: KeyInput, out: &mut Vec<u8>) {
        if key.unit == 0 || !(key.key_down || key.virtual_key == VK_MENU) {
            return;
        }
        let (lead, character) = match key.unit {
            0xD800..=0xDBFF => (self.high.replace(key.unit).map(|_| REPLACEMENT), None),
            0xDC00..=0xDFFF => match self.high.take() {
                Some(high) => (None, Some(join_pair(high, key.unit))),
                None => (None, Some(REPLACEMENT)),
            },
            unit => (
                self.high.take().map(|_| REPLACEMENT),
                Some(char::from_u32(u32::from(unit)).unwrap_or(REPLACEMENT)),
            ),
        };
        let mut buffer = [0u8; 4];
        if let Some(lead) = lead {
            out.extend_from_slice(lead.encode_utf8(&mut buffer).as_bytes());
        }
        if let Some(character) = character {
            let bytes = character.encode_utf8(&mut buffer).as_bytes();
            for _ in 0..key.repeat.max(1) {
                out.extend_from_slice(bytes);
            }
        }
    }
}

fn join_pair(high: u16, low: u16) -> char {
    std::char::decode_utf16([high, low])
        .next()
        .and_then(Result::ok)
        .unwrap_or(REPLACEMENT)
}

/// Decodes the remote terminal's byte stream as UTF-8 across chunk borders: an
/// incomplete sequence at the end of a chunk waits for the next one, invalid
/// bytes become U+FFFD.
#[derive(Default)]
struct Utf8Decoder {
    pending: Vec<u8>,
}

impl Utf8Decoder {
    fn push(&mut self, chunk: &[u8]) -> String {
        let mut data = std::mem::take(&mut self.pending);
        data.extend_from_slice(chunk);
        let mut text = String::with_capacity(data.len());
        let mut rest = data.as_slice();
        while let Err(error) = std::str::from_utf8(rest) {
            let (valid, after) = rest.split_at(error.valid_up_to());
            text.push_str(&String::from_utf8_lossy(valid));
            let Some(invalid) = error.error_len() else {
                self.pending = after.to_vec();
                return text;
            };
            text.push(REPLACEMENT);
            rest = &after[invalid..];
        }
        text.push_str(&String::from_utf8_lossy(rest));
        text
    }
}

#[cfg(test)]
mod tests {
    use super::{
        raw_input_mode, vt_output_mode, KeyInput, KeyTranslator, Utf8Decoder,
        DISABLE_NEWLINE_AUTO_RETURN, ENABLE_ECHO_INPUT, ENABLE_LINE_INPUT, ENABLE_PROCESSED_INPUT,
        ENABLE_PROCESSED_OUTPUT, ENABLE_VIRTUAL_TERMINAL_INPUT, ENABLE_VIRTUAL_TERMINAL_PROCESSING,
        ENABLE_WINDOW_INPUT, VK_MENU,
    };

    fn key(unit: u16, key_down: bool, repeat: u16, virtual_key: u16) -> KeyInput {
        KeyInput {
            key_down,
            repeat,
            virtual_key,
            unit,
        }
    }

    fn typed(keys: &[KeyInput]) -> Vec<u8> {
        let mut translator = KeyTranslator::default();
        let mut out = Vec::new();
        keys.iter().for_each(|key| translator.push(*key, &mut out));
        out
    }

    fn press(unit: u16) -> KeyInput {
        key(unit, true, 1, 0x41)
    }

    #[test]
    fn terminal_task_windows_console_modes_follow_the_ssh_recipe() {
        let input = raw_input_mode(0x1F7); // the usual input mode, with quick edit (0x40)
        for on in [ENABLE_VIRTUAL_TERMINAL_INPUT, ENABLE_WINDOW_INPUT, 0x40] {
            assert_eq!(input & on, on);
        }
        for off in [ENABLE_PROCESSED_INPUT, ENABLE_LINE_INPUT, ENABLE_ECHO_INPUT] {
            assert_eq!(input & off, 0);
        }
        let on = [ENABLE_PROCESSED_OUTPUT, ENABLE_VIRTUAL_TERMINAL_PROCESSING];
        let output = vt_output_mode(0x3, true);
        for flag in on.into_iter().chain([DISABLE_NEWLINE_AUTO_RETURN]) {
            assert_ne!(output & flag, 0);
        }
        assert_eq!(vt_output_mode(0x3, false) & DISABLE_NEWLINE_AUTO_RETURN, 0);
    }

    #[test]
    fn terminal_task_windows_keys_become_utf8_with_repeats_and_surrogates() {
        assert_eq!(typed(&[press(u16::from(b'a')), press(0x0D)]), b"a\r");
        assert_eq!(typed(&[press(0x20AC)]), "\u{20AC}".as_bytes());
        assert_eq!(typed(&[key(u16::from(b'x'), true, 3, 0x58)]), b"xxx");
        assert_eq!(typed(&[key(u16::from(b'y'), true, 0, 0x59)]), b"y");
        // U+1F600 is the pair D83D DE00 in two records; lone surrogates are U+FFFD.
        assert_eq!(
            typed(&[press(0xD83D), press(0xDE00)]),
            "\u{1F600}".as_bytes()
        );
        assert_eq!(typed(&[press(0xDE00)]), "\u{FFFD}".as_bytes());
        let lone = typed(&[press(0xD83D), press(0xD83D), press(0xDE00), press(0xD83D)]);
        assert_eq!(lone, "\u{FFFD}\u{1F600}".as_bytes());
        // Releases and empty characters type nothing, except the release of Alt.
        assert!(typed(&[key(u16::from(b'a'), false, 1, 0x41), press(0)]).is_empty());
        assert_eq!(typed(&[key(u16::from(b'A'), false, 1, VK_MENU)]), b"A");
        assert!(typed(&[key(0, false, 1, VK_MENU)]).is_empty());
    }

    #[test]
    fn terminal_task_windows_output_decodes_utf8_across_chunks() {
        let mut decoder = Utf8Decoder::default();
        assert_eq!(decoder.push(b"plain"), "plain");
        // The euro sign E2 82 AC in three chunks, then U+1F600 as F0 9F 98 80.
        let chunks: [&[u8]; 6] = [&[0xE2], &[0x82], &[0xAC], b"ok\xF0", &[0x9F, 0x98], &[0x80]];
        let text: Vec<String> = chunks.iter().map(|chunk| decoder.push(chunk)).collect();
        assert_eq!(text, ["", "", "\u{20AC}", "ok", "", "\u{1F600}"]);
        // Invalid bytes become U+FFFD; the text around them survives.
        assert_eq!(decoder.push(&[0xFF, b'a']), "\u{FFFD}a");
        assert_eq!(decoder.push(&[0xE2]), "");
        assert_eq!(decoder.push(b"x"), "\u{FFFD}x");
        assert_eq!(decoder.push(b"\x1b[31m"), "\u{1b}[31m");
    }
}
