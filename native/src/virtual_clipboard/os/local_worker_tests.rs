//! M17 acceptance: a local virtual-files paste is served by the data
//! object's own STA thread. Another apartment (standing in for Explorer)
//! reads a file's IStream while the thread that put it on the clipboard (the
//! GUI thread) is blocked without pumping messages.
use super::super::{place_files, register, VirtualFile};
use std::sync::mpsc;
use std::time::{Duration, Instant};
use windows::core::{Error, Result};
use windows::Win32::Foundation::{E_UNEXPECTED, S_FALSE};
use windows::Win32::System::Com::{
    IDataObject, IStream, DVASPECT_CONTENT, FORMATETC, TYMED_ISTREAM,
};
use windows::Win32::System::DataExchange::GetClipboardSequenceNumber;
use windows::Win32::System::Ole::{
    OleGetClipboard, OleInitialize, OleSetClipboard, OleUninitialize, ReleaseStgMedium,
};
use windows::Win32::System::Threading::GetCurrentThreadId;

/// Generous for slow runners; waits end as soon as the condition holds.
const DEADLINE: Duration = Duration::from_secs(60);

/// OLE on the calling thread (an STA, like the GUI thread).
struct Apartment;

impl Apartment {
    fn enter() -> Self {
        unsafe { OleInitialize(None) }.expect("OLE initializes on this thread");
        Self
    }
}

impl Drop for Apartment {
    fn drop(&mut self) {
        unsafe { OleUninitialize() };
    }
}

fn first_stream(object: &IDataObject) -> Result<IStream> {
    let request = FORMATETC {
        cfFormat: register("FileContents"),
        ptd: std::ptr::null_mut(),
        dwAspect: DVASPECT_CONTENT.0,
        lindex: 0,
        tymed: TYMED_ISTREAM.0 as u32,
    };
    let mut medium = unsafe { object.GetData(&request)? };
    let stream = unsafe { (*medium.u.pstm).clone() };
    unsafe { ReleaseStgMedium(&mut medium) };
    stream.ok_or_else(|| Error::from(E_UNEXPECTED))
}

/// Reads until the short read that ends the stream, in odd-sized chunks.
fn read_all(stream: &IStream) -> Result<Vec<u8>> {
    let mut data = Vec::new();
    let mut chunk = [0u8; 7_919];
    loop {
        let mut read = 0u32;
        let status = unsafe {
            stream.Read(
                chunk.as_mut_ptr().cast(),
                chunk.len() as u32,
                Some(&mut read),
            )
        };
        status.ok()?;
        data.extend_from_slice(&chunk[..read as usize]);
        if status == S_FALSE {
            return Ok(data);
        }
    }
}

/// What Explorer does on paste, from its own apartment.
fn paste_first_file() -> Result<Vec<u8>> {
    let _apartment = Apartment::enter();
    let object = unsafe { OleGetClipboard()? };
    read_all(&first_stream(&object)?)
}

#[test]
fn speed_task_local_clipboard_streams_are_read_while_the_gui_thread_is_busy() {
    let fixture = tempfile::tempdir().unwrap();
    let path = fixture.path().join("datei.bin");
    let bytes: Vec<u8> = (0..300_000u32).map(|i| (i % 251) as u8).collect();
    std::fs::write(&path, &bytes).unwrap();
    let gui = Apartment::enter();
    let placed = place_files(vec![VirtualFile {
        abs: path.to_string_lossy().into_owned(),
        rel: "Ordner/datei.bin".into(),
        size: bytes.len() as u64,
        mtime_ms: 0,
    }])
    .expect("virtual files on the clipboard");
    assert_eq!(placed.sequence, unsafe { GetClipboardSequenceNumber() });
    assert_ne!(
        placed.thread(),
        unsafe { GetCurrentThreadId() },
        "the data object lives on its own thread"
    );

    let (sent, received) = mpsc::channel();
    let reader = std::thread::spawn(move || {
        let _ = sent.send(paste_first_file().map_err(|error| error.to_string()));
    });
    // The GUI thread is busy: it blocks without pumping messages. With the
    // object on this thread, the read above could not finish meanwhile.
    let pasted = received
        .recv_timeout(DEADLINE)
        .expect("the paste reads while the GUI thread pumps no messages");
    assert_eq!(pasted.expect("stream read"), bytes);
    reader.join().unwrap();

    // Another owner empties the clipboard: OLE releases the object and its
    // worker thread ends without rendering anything.
    unsafe { OleSetClipboard(None::<&IDataObject>) }.expect("clipboard emptied");
    let deadline = Instant::now() + DEADLINE;
    while !placed.exited() {
        assert!(Instant::now() < deadline, "the worker thread ends");
        std::thread::sleep(Duration::from_millis(5));
    }
    assert_eq!(std::fs::read(&path).unwrap(), bytes, "the source stays");
    drop(gui);
}
