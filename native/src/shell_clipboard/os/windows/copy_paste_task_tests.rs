//! Actual Windows clipboard acceptance; the task entrypoint serializes these.
use std::{
    sync::mpsc::{self, Receiver, Sender},
    thread::{self, JoinHandle},
    time::{Duration, Instant},
};
use windows::{core::Result, Win32::{
    Foundation::HGLOBAL,
    System::{
        DataExchange::{EmptyClipboard, GetClipboardData, GetClipboardSequenceNumber},
        Memory::{GlobalLock, GlobalSize, GlobalUnlock},
        Ole::CF_HDROP,
    },
}};
use super::super::{
    header, memory::{LockedGlobal, OwnedGlobal}, owner::Clipboard,
    preferred_drop_effect_fmt, read_files, write_files, write_files_if_sequence,
    DROPEFFECT_COPY, DROPEFFECT_MOVE,
};

const WAIT: Duration = Duration::from_secs(5);

struct IsolatedClipboard;

impl IsolatedClipboard {
    fn enter() -> Self {
        assert_eq!(std::env::var("SMART_EXPLORER_COPY_PASTE_TASK").as_deref(), Ok("1"),
            "Clipboard acceptance requires the isolated, sequential remote task runner");
        clear().expect("Cannot initialize the isolated Windows clipboard");
        Self
    }
}

impl Drop for IsolatedClipboard {
    fn drop(&mut self) {
        if let Err(error) = clear() {
            eprintln!("copy_paste_task clipboard cleanup failed: {error}");
        }
    }
}

fn clear() -> Result<()> {
    let clipboard = Clipboard::open()?;
    unsafe { EmptyClipboard()?; }
    clipboard.close()
}

fn sequence() -> u32 {
    let value = unsafe { GetClipboardSequenceNumber() };
    assert_ne!(value, 0, "Clipboard sequence is unavailable on the isolated runner");
    value
}

// No assertion can leave a clipboard-holding worker behind. Both its receive
// and our reap have deadlines; an unreapable native call terminates this task
// process instead of allowing another case to touch an ambiguously held lock.
struct HeldClipboard {
    release: Option<Sender<()>>,
    done: Receiver<std::result::Result<(), String>>,
    worker: Option<JoinHandle<()>>,
}

impl HeldClipboard {
    fn start() -> Self {
        let (ready_tx, ready_rx) = mpsc::channel::<std::result::Result<(), String>>();
        let (release_tx, release_rx) = mpsc::channel();
        let (done_tx, done_rx) = mpsc::channel();
        let worker = thread::Builder::new().name("clipboard-contention".into()).spawn(move || {
            let result = (|| -> std::result::Result<(), String> {
                let clipboard = Clipboard::open().map_err(|error| error.to_string())?;
                ready_tx.send(Ok(())).map_err(|error| error.to_string())?;
                let release = release_rx.recv_timeout(WAIT * 2);
                clipboard.close().map_err(|error| error.to_string())?;
                release.map_err(|error| error.to_string())
            })();
            if let Err(error) = &result { let _ = ready_tx.send(Err(error.clone())); }
            let _ = done_tx.send(result);
        }).expect("Cannot create bounded clipboard holder");
        let mut holder = Self { release: Some(release_tx), done: done_rx, worker: Some(worker) };
        let ready = ready_rx.recv_timeout(WAIT);
        if !matches!(&ready, Ok(Ok(()))) {
            let cleanup = holder.finish();
            panic!("Clipboard holder did not acquire the lock: {ready:?}; cleanup: {cleanup:?}");
        }
        holder
    }

    fn finish(&mut self) -> std::result::Result<(), String> {
        if let Some(release) = self.release.take() { let _ = release.send(()); }
        let Some(worker) = self.worker.as_ref() else { return Ok(()); };
        let deadline = Instant::now() + WAIT;
        while !worker.is_finished() {
            if Instant::now() >= deadline {
                eprintln!("copy_paste_task clipboard holder could not be reaped within five seconds");
                std::process::abort();
            }
            thread::sleep(Duration::from_millis(5));
        }
        self.worker.take().expect("Holder has a worker").join()
            .map_err(|_| "Clipboard holder panicked".to_owned())?;
        self.done.try_recv().map_err(|error| error.to_string())?
    }
}

impl Drop for HeldClipboard {
    fn drop(&mut self) {
        if let Err(error) = self.finish() {
            eprintln!("copy_paste_task clipboard holder cleanup failed: {error}");
        }
    }
}

fn assert_lock_released() {
    // A different thread/window must open it; reopening from the same owner is
    // not sufficient evidence that an earlier OpenClipboard was closed.
    HeldClipboard::start().finish().expect("Reader did not release the clipboard");
}

fn assert_selection(paths: &[String], is_cut: bool) {
    assert_eq!(read_files().expect("Clipboard read failed"), Some((paths.to_vec(), is_cut)));
}

#[test]
#[ignore = "requires isolated remote copy/paste task runner"]
fn copy_paste_task_windows_roundtrip_copy_move() {
    let _isolated = IsolatedClipboard::enter();
    let paths = [
        "C:\\Copy Paste\\ Mixed Case 日本語 😀.txt",
        "C:\\Copy Paste\\ leading and trailing .txt ",
        "\\\\server\\share\\ folder \\résumé.txt",
        "\\\\?\\C:\\copy-paste\\𝄞.txt",
        "\\\\?\\UNC\\server\\share\\資料.txt",
    ].map(str::to_owned).to_vec();
    for _ in 0..4 {
        for effect in [DROPEFFECT_COPY, DROPEFFECT_MOVE] {
            write_files(&paths, effect).expect("Owned-window clipboard publication failed");
            assert_selection(&paths, effect == DROPEFFECT_MOVE);
        }
    }
    assert_lock_released();
    clear().unwrap();
}

#[test]
#[ignore = "requires isolated remote copy/paste task runner"]
fn copy_paste_task_windows_guarded_sequence() {
    let _isolated = IsolatedClipboard::enter();
    let old = vec!["C:\\old.txt".to_owned()];
    let newer = vec!["C:\\newer 日本語.txt".to_owned()];
    let prepared = vec!["\\\\server\\share\\prepared.txt".to_owned()];
    write_files(&old, DROPEFFECT_COPY).unwrap();
    let stale = sequence();
    write_files(&newer, DROPEFFECT_COPY).unwrap();
    let current = sequence();
    assert_ne!(current, stale);
    assert_eq!(write_files_if_sequence(&prepared, DROPEFFECT_MOVE, stale).unwrap(), None);
    assert_eq!(sequence(), current, "A stale publication changed the newer clipboard");
    assert_selection(&newer, false);
    assert!(write_files_if_sequence(&prepared, DROPEFFECT_MOVE, 0).is_err());
    assert_eq!(sequence(), current);
    assert_selection(&newer, false);
    let published = write_files_if_sequence(&prepared, DROPEFFECT_MOVE, current)
        .unwrap().expect("Matching publication was rejected");
    assert_ne!(published, current);
    assert_eq!(published, sequence());
    assert_selection(&prepared, true);
    assert_lock_released();
    clear().unwrap();
}

#[test]
#[ignore = "requires isolated remote copy/paste task runner"]
fn copy_paste_task_windows_contention_and_recovery() {
    let _isolated = IsolatedClipboard::enter();
    assert_eq!(read_files().unwrap(), None, "An empty clipboard has no CF_HDROP");
    let mut holder = HeldClipboard::start();
    let busy_read = read_files();
    let busy_write = write_files(&["C:\\busy.txt".to_owned()], DROPEFFECT_COPY);
    // Release/join before asserting outcomes, including unexpected success.
    holder.finish().expect("Clipboard holder failed to close");
    assert!(busy_read.is_err(), "Contention was reported as empty/success: {busy_read:?}");
    assert!(busy_write.is_err(), "Publication succeeded while another window held the clipboard");
    assert_eq!(read_files().unwrap(), None);
    let recovered = vec!["C:\\recovered.txt".to_owned()];
    write_files(&recovered, DROPEFFECT_MOVE).unwrap();
    assert_selection(&recovered, true);
    clear().unwrap();
}

fn wide_payload(path: &str, terminators: usize) -> Vec<u8> {
    let mut bytes = header(true);
    for unit in path.encode_utf16().chain(std::iter::repeat(0).take(terminators)) {
        bytes.extend_from_slice(&unit.to_le_bytes());
    }
    bytes
}

fn payload_global(bytes: &[u8]) -> OwnedGlobal {
    let owned = OwnedGlobal::from_bytes(bytes).unwrap();
    let size = unsafe { GlobalSize(owned.handle()) };
    assert!(size >= bytes.len());
    let pointer = unsafe { GlobalLock(owned.handle()) }.cast::<u8>();
    assert!(!pointer.is_null(), "Cannot initialize owned clipboard fixture padding");
    // GlobalAlloc may round upward. Prevent zero-initialized padding from
    // accidentally supplying the very terminators a malformed fixture omits.
    // No operation between this lock and unlock can panic or transfer ownership.
    unsafe {
        std::ptr::write_bytes(pointer.add(bytes.len()), 0x7f, size - bytes.len());
        let _ = GlobalUnlock(owned.handle());
    }
    owned
}

fn publish_raw(bytes: &[u8], effect: Option<u32>) -> Result<()> {
    let files = payload_global(bytes);
    let preferred = effect.map(|value| OwnedGlobal::from_bytes(&value.to_le_bytes())).transpose()?;
    let format = preferred_drop_effect_fmt()?;
    let clipboard = Clipboard::open()?;
    unsafe { EmptyClipboard()?; }
    if let Some(preferred) = preferred { preferred.publish(format)?; }
    files.publish(CF_HDROP.0 as u32)?;
    clipboard.close()
}

#[test]
#[ignore = "requires isolated remote copy/paste task runner"]
fn copy_paste_task_windows_malformed_dropfiles_and_recovery() {
    let _isolated = IsolatedClipboard::enter();
    let mut inside_header = wide_payload("C:\\offset.txt", 2);
    inside_header[..4].copy_from_slice(&0u32.to_le_bytes());
    let mut outside_allocation = inside_header.clone();
    outside_allocation[..4].copy_from_slice(&u32::MAX.to_le_bytes());
    let mut surrogate = wide_payload("C:\\", 0);
    for unit in [0xd800u16, 0, 0] { surrogate.extend_from_slice(&unit.to_le_bytes()); }
    let cases = [
        ("short header", vec![0x7f; 4]),
        ("offset inside header", inside_header),
        ("offset beyond allocation", outside_allocation),
        ("missing path terminator", wide_payload("C:\\missing.txt", 0)),
        ("missing list terminator", wide_payload("C:\\single.txt", 1)),
        ("empty list missing second terminator", wide_payload("", 1)),
        ("unpaired UTF-16 surrogate", surrogate),
        ("relative path", wide_payload("relative\\note.md", 2)),
    ];
    let recovered = vec!["C:\\valid after malformed.txt".to_owned()];
    for (label, bytes) in cases {
        publish_raw(&bytes, None).unwrap();
        let observed = read_files();
        assert_lock_released();
        assert!(observed.is_err(), "Malformed {label} was accepted: {observed:?}");
        write_files(&recovered, DROPEFFECT_COPY).unwrap();
        assert_selection(&recovered, false);
    }
    clear().unwrap();
}

fn read_effect_prefix(length: usize) -> Result<bool> {
    let clipboard = Clipboard::open()?;
    let handle = unsafe { GetClipboardData(preferred_drop_effect_fmt()?)? };
    let locked = unsafe { LockedGlobal::new(HGLOBAL(handle.0))? };
    let effect = super::decode_effect(&locked.bytes()[..length])?;
    drop(locked);
    clipboard.close()?;
    Ok(effect)
}

#[test]
#[ignore = "requires isolated remote copy/paste task runner"]
fn copy_paste_task_windows_undersized_effect_and_recovery() {
    let _isolated = IsolatedClipboard::enter();
    let paths = vec!["C:\\effect.txt".to_owned()];
    write_files(&paths, DROPEFFECT_MOVE).unwrap();
    // Requested allocation lengths are not the GlobalSize seen by read_files.
    // Exercise exact 0..3-byte prefixes of real locked clipboard data through
    // the private parser, retaining the same early-error RAII ownership chain.
    for length in 0..4 {
        let observed = read_effect_prefix(length);
        assert_lock_released();
        assert!(observed.is_err(), "Accepted a {length}-byte drop effect");
        assert_selection(&paths, true);
    }
    assert!(read_effect_prefix(4).unwrap());
    clear().unwrap();
}

// Independent ANSI oracle; exact Microsoft signature, no production/Cargo
// change. CP_ACP=0 follows the runner's actual Windows code page, not UTF-8 or
// an assumed English locale. Positive input length excludes the terminator.
#[link(name = "kernel32")]
unsafe extern "system" {
    fn MultiByteToWideChar(code_page: u32, flags: u32, source: *const u8,
        source_length: i32, destination: *mut u16, destination_length: i32) -> i32;
}

fn windows_ansi(path: &[u8]) -> String {
    let length = i32::try_from(path.len()).unwrap();
    let needed = unsafe { MultiByteToWideChar(0, 0, path.as_ptr(), length, std::ptr::null_mut(), 0) };
    assert!(needed > 0, "Windows ANSI oracle could not size the fixture conversion");
    let mut units = vec![0; needed as usize];
    let written = unsafe { MultiByteToWideChar(0, 0, path.as_ptr(), length, units.as_mut_ptr(), needed) };
    assert_eq!(written, needed);
    String::from_utf16(&units).unwrap()
}

#[test]
#[ignore = "requires isolated remote copy/paste task runner"]
fn copy_paste_task_windows_ansi_hdrop_conversion() {
    let _isolated = IsolatedClipboard::enter();
    let paths: [&[u8]; 3] = [
        b"C:\\ANSI folder\\caf\xe9.txt",
        b"\\\\server\\share\\r\xe9sum\xe9.txt",
        b"C:\\ANSI folder\\two-byte-\xc3\xa9.txt",
    ];
    let expected: Vec<String> = paths.iter().map(|path| windows_ansi(path)).collect();
    let mut bytes = header(false);
    for path in paths { bytes.extend_from_slice(path); bytes.push(0); }
    bytes.push(0);
    publish_raw(&bytes, Some(DROPEFFECT_MOVE)).unwrap();
    assert_selection(&expected, true);
    assert_lock_released();
    clear().unwrap();
}
