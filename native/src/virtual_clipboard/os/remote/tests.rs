//! Remote virtual files, exercised the way Explorer uses them: through a
//! marshaled proxy of the worker's object (drag), directly (clipboard paste
//! semantics) and, on the isolated clipboard runner, via the real clipboard.
use super::catalog::{Catalog, TOO_LARGE_NOTE};
use super::data_object::{Formats, RemoteDataObject};
use super::handoff::{Config, Handoff};
use super::test_support::*;
use super::worker::LifeToken;
use super::{set_clipboard_with, start_drag_with};
use crate::transfer::{ListedEntry, SelectionListing};
use std::sync::Arc;
use std::time::Duration;
use windows::core::{Error, Interface, Result};
use windows::Win32::Foundation::{
    E_OUTOFMEMORY, HGLOBAL, STG_E_ACCESSDENIED, STG_E_MEDIUMFULL, STG_E_READFAULT, S_FALSE, S_OK,
};
use windows::Win32::Storage::FileSystem::FILE_ATTRIBUTE_DIRECTORY;
use windows::Win32::System::Com::{
    IBindCtx, IDataObject, IStream, STATFLAG_NONAME, STATSTG, STREAM_SEEK_CUR, STREAM_SEEK_END,
    STREAM_SEEK_SET, TYMED_HGLOBAL,
};
use windows::Win32::System::DataExchange::GetClipboardSequenceNumber;
use windows::Win32::System::Ole::{OleGetClipboard, OleSetClipboard, DROPEFFECT_COPY};
use windows::Win32::UI::Shell::{
    IDataObjectAsyncCapability, FD_ATTRIBUTES, FD_FILESIZE, FD_PROGRESSUI, FD_WRITESTIME,
};

const MTIME_MS: i64 = 1_700_000_000_000;
/// FILETIME ticks of MTIME_MS: 100 ns steps since 1601.
const MTIME_TICKS: u64 = (1_700_000_000_000 + 11_644_473_600_000) * 10_000;

fn has(flags: u32, flag: i32) -> bool {
    flags & flag as u32 != 0
}

fn in_process(remote: FakeRemote, config: Config) -> IDataObject {
    let handoff = Handoff::new(Arc::new(remote), config).expect("hand-off state");
    RemoteDataObject::new(handoff.clone(), LifeToken::detached(handoff)).into()
}

fn stat_size(stream: &IStream) -> u64 {
    let mut stat = STATSTG::default();
    unsafe { stream.Stat(&mut stat, STATFLAG_NONAME) }.expect("Stat");
    stat.cbSize
}

fn seek(stream: &IStream, offset: i64, origin: windows::Win32::System::Com::STREAM_SEEK) -> u64 {
    let mut position = 0u64;
    unsafe { stream.Seek(offset, origin, Some(&mut position)) }.expect("Seek");
    position
}

#[test]
fn transfer_engine_task_remote_catalog_keeps_windows_limits() {
    let entry = |rel: &str, is_dir: bool| ListedEntry {
        rel: rel.to_string(),
        path: format!("/r/{rel}"),
        id: None,
        size: 1,
        size_known: true,
        mtime_ms: 0,
        is_dir,
    };
    let edge = format!("a/{}", "e".repeat(257));
    let over = format!("a/{}", "o".repeat(258));
    assert_eq!(
        (edge.encode_utf16().count(), over.encode_utf16().count()),
        (259, 260)
    );
    // Names a server may send that Explorer would turn into other paths.
    let unsafe_names = [
        "a/back\\slash.txt",
        "a/../raus.txt",
        "a/datei.txt:strom",
        "a/tab\tname",
        "a/CON.txt",
        "a/com1",
        "a/ende.",
    ];
    let mut entries = vec![entry("a", true), entry(&edge, false), entry(&over, false)];
    entries.extend(unsafe_names.iter().copied().map(|rel| entry(rel, false)));
    entries.push(entry("a/.versteckt", false));
    entries.push(entry("a/COM10.txt", false));
    let catalog = Catalog::from_listing(SelectionListing {
        entries,
        problems: vec![("/r/b".to_string(), "Zugriff verweigert".to_string())],
        omitted: 0,
        complete: true,
    });
    let kept: Vec<&str> = catalog.entries.iter().map(|e| e.rel.as_str()).collect();
    assert_eq!(kept, ["a", edge.as_str(), "a/.versteckt", "a/COM10.txt"]);
    assert_eq!(catalog.files, 3);
    assert_eq!(catalog.too_long, [over]);
    let refused: Vec<&str> = catalog.problems[1..]
        .iter()
        .map(|(path, _)| path.as_str())
        .collect();
    let expected: Vec<String> = unsafe_names.iter().map(|rel| format!("/r/{rel}")).collect();
    assert_eq!(
        refused, expected,
        "every unsafe name is reported, none handed over"
    );
    let reserved = &catalog.problems[5].1;
    assert!(
        reserved.contains("in Smart Explorer einfügen"),
        "{reserved}"
    );
    assert_eq!(catalog.errors(), 1 + 1 + unsafe_names.len() as u64);
    let note = catalog.note().expect("a note explains the omission");
    assert!(
        note.starts_with("1 Eintrag mit Pfaden ab 260 Zeichen"),
        "{note}"
    );
    assert!(
        note.ends_with("in Smart Explorer einfügen überträgt sie"),
        "{note}"
    );
}

#[test]
fn transfer_engine_task_remote_handoff_ole_roundtrip_lists_and_streams() {
    let _apartment = Apartment::enter();
    let label = "transfer_engine_task roundtrip";
    let alpha: Vec<u8> = (0..300_000u32).map(|i| (i % 251) as u8).collect();
    let edge = format!("Projekt/{}", "e".repeat(251));
    let over = format!("Projekt/{}", "o".repeat(252));
    let remote = FakeRemote::new(label)
        .dir("Projekt")
        .file("Projekt/alpha.bin", alpha.clone(), MTIME_MS)
        .file("Projekt/leer.txt", Vec::new(), MTIME_MS)
        .export("Projekt/Bericht.docx", vec![7; 5_000])
        .file(&edge, b"edge".to_vec(), 0)
        .file(&over, b"zu lang".to_vec(), 0)
        .dir("Projekt/Leer")
        .problem("/fake/Projekt/gesperrt", "Zugriff verweigert");
    let stats = remote.stats.clone();
    let mut drag = start_drag_with(Arc::new(remote), Config::default()).expect("worker starts");
    let control = drag.control();
    let object = drag.data_object().expect("proxy on the test thread");
    let formats = Formats::register();
    let query = request(formats.descriptor, TYMED_HGLOBAL, -1);
    assert_eq!(unsafe { object.QueryGetData(&query) }, S_OK);
    assert!(unsafe { object.EnumFormatEtc(1) }.is_ok());
    assert_eq!(
        stats.listings(),
        0,
        "nothing is listed before Explorer asks"
    );

    let listed = descriptor(&object, &formats).expect("descriptor");
    assert_eq!(descriptor(&object, &formats).expect("again"), listed);
    assert_eq!(stats.listings(), 1, "listed once, then cached");
    let names: Vec<&str> = listed.iter().map(|d| d.name.as_str()).collect();
    let edge_name = edge.replace('/', "\\");
    assert_eq!(
        names,
        [
            "Projekt",
            "Projekt\\alpha.bin",
            "Projekt\\leer.txt",
            "Projekt\\Bericht.docx",
            edge_name.as_str(),
            "Projekt\\Leer",
        ]
    );
    let (folder, file, empty, export) = (&listed[0], &listed[1], &listed[2], &listed[3]);
    assert!(has(folder.flags, FD_ATTRIBUTES.0) && has(folder.flags, FD_PROGRESSUI.0));
    assert_eq!(folder.attributes, FILE_ATTRIBUTE_DIRECTORY.0);
    assert!(
        !has(folder.flags, FD_WRITESTIME.0),
        "unknown folder times stay unset"
    );
    assert!(has(file.flags, FD_FILESIZE.0) && has(file.flags, FD_WRITESTIME.0));
    assert_eq!(
        (file.size, file.write_time),
        (alpha.len() as u64, MTIME_TICKS)
    );
    assert!(
        has(empty.flags, FD_FILESIZE.0),
        "size 0 creates an empty file"
    );
    assert_eq!(empty.size, 0);
    assert!(
        !has(export.flags, FD_FILESIZE.0),
        "unknown sizes are not claimed"
    );

    // Folders never get contents requests from Explorer; answer them empty.
    assert_eq!(
        read_all(&contents(&object, &formats, 0).unwrap()).unwrap(),
        b""
    );
    assert_eq!(
        read_all(&contents(&object, &formats, 1).unwrap()).unwrap(),
        alpha
    );
    assert_eq!(
        read_all(&contents(&object, &formats, 2).unwrap()).unwrap(),
        b""
    );
    let report = contents(&object, &formats, 3).unwrap();
    assert_eq!(
        stat_size(&report),
        5_000,
        "Stat reads an export to learn its size"
    );
    assert_eq!(read_all(&report).unwrap(), vec![7; 5_000]);
    drop(report);
    let out_of_range = contents(&object, &formats, 6).map(|_| ()).unwrap_err();
    assert_eq!(out_of_range.code(), windows::Win32::Foundation::DV_E_LINDEX);
    assert_eq!(
        read_all(&contents(&object, &formats, 4).unwrap()).unwrap(),
        b"edge"
    );

    let entry = external(label);
    assert_eq!((entry.files_total, entry.files_done), (4, 4));
    assert_eq!(entry.bytes_done, alpha.len() as u64 + 5_000 + 4);
    assert_eq!(entry.errors, 2, "the long path and the listing problem");
    assert!(entry.finished, "the paste ends once every file arrived");
    let note = entry.note.unwrap_or_default();
    assert!(
        note.starts_with("1 Eintrag mit Pfaden ab 260 Zeichen"),
        "{note}"
    );

    drop(object);
    drop(drag);
    wait_until("the worker thread ends", || control.exited());
}

fn refuse(_bytes: usize) -> Result<HGLOBAL> {
    Err(Error::from(E_OUTOFMEMORY))
}

#[test]
fn transfer_engine_task_remote_descriptor_alloc_failure_is_medium_full() {
    let _apartment = Apartment::enter();
    let label = "transfer_engine_task too large";
    let remote = FakeRemote::new(label).file("a.txt", b"a".to_vec(), 0);
    let config = Config {
        alloc: refuse,
        ..Config::default()
    };
    let mut drag = start_drag_with(Arc::new(remote), config).expect("worker starts");
    let control = drag.control();
    let object = drag.data_object().expect("proxy");
    let formats = Formats::register();
    for _ in 0..2 {
        let error = descriptor(&object, &formats).unwrap_err();
        assert_eq!(error.code(), STG_E_MEDIUMFULL);
    }
    let entry = external(label);
    assert_eq!(entry.note.as_deref(), Some(TOO_LARGE_NOTE));
    assert_eq!(entry.errors, 1, "one paste, one error");
    drop(object);
    drop(drag);
    wait_until("the worker thread ends", || control.exited());
}

#[test]
fn transfer_engine_task_remote_stream_seek_clone_stat_and_read_errors() {
    let _apartment = Apartment::enter();
    let label = "transfer_engine_task stream";
    let data: Vec<u8> = (0..100_000u32).map(|i| (i * 7 % 256) as u8).collect();
    let remote = FakeRemote::new(label)
        .file("daten.bin", data.clone(), 0)
        .failing("kaputt.bin", vec![1; 50_000], 10_000);
    let object = in_process(remote, Config::default());
    let formats = Formats::register();
    assert_eq!(descriptor(&object, &formats).unwrap().len(), 2);

    let stream = contents(&object, &formats, 0).unwrap();
    assert_eq!(read_exact(&stream, 10), data[..10]);
    assert_eq!(seek(&stream, 0, STREAM_SEEK_CUR), 10);
    assert_eq!(seek(&stream, 0, STREAM_SEEK_SET), 0);
    assert_eq!(read_exact(&stream, 10), data[..10], "rewind fetches again");
    assert_eq!(seek(&stream, 50_000, STREAM_SEEK_SET), 50_000);
    assert_eq!(
        read_exact(&stream, 10),
        data[50_000..50_010],
        "forward seek skips"
    );
    let clone = unsafe { stream.Clone() }.expect("Clone");
    assert_eq!(read_exact(&clone, 10), data[50_010..50_020]);
    assert_eq!(
        read_exact(&stream, 10),
        data[50_010..50_020],
        "clones read apart"
    );
    assert_eq!(seek(&stream, 0, STREAM_SEEK_END), 100_000);
    let mut byte = [0u8; 1];
    let mut read = 1u32;
    let status = unsafe { stream.Read(byte.as_mut_ptr().cast(), 1, Some(&mut read)) };
    assert_eq!(
        (status, read),
        (S_FALSE, 0),
        "reading at the end yields nothing"
    );
    assert_eq!(stat_size(&stream), 100_000);
    let mut written = 9u32;
    let status = unsafe { stream.Write(b"x".as_ptr().cast(), 1, Some(&mut written)) };
    assert_eq!((status, written), (STG_E_ACCESSDENIED, 0));
    assert!(unsafe { stream.SetSize(0) }.is_err());
    assert!(unsafe { stream.Seek(-1, STREAM_SEEK_SET, None) }.is_err());
    drop((stream, clone));

    let broken = contents(&object, &formats, 1).unwrap();
    let error = read_all(&broken).unwrap_err();
    assert_eq!(error.code(), STG_E_READFAULT);
    let entry = external(label);
    assert_eq!(entry.errors, 1);
    assert_eq!(
        entry.note.as_deref(),
        Some("kaputt.bin: Verbindung getrennt")
    );
}

#[test]
fn transfer_engine_task_remote_prefetch_runs_ahead_within_budget_and_flow() {
    let _apartment = Apartment::enter();
    let label = "transfer_engine_task prefetch";
    const SIZE: usize = 20_000;
    let name = |n: usize| format!("f{n}");
    let files: Vec<Vec<u8>> = (0..8u8).map(|n| vec![n; SIZE]).collect();
    // File 0 reads only once file 1 was opened, which only prefetch does.
    let mut remote = FakeRemote::new(label)
        .ceiling(2)
        .gated(&name(0), files[0].clone(), &name(1));
    for (n, bytes) in files.iter().enumerate().skip(1) {
        remote = remote.file(&name(n), bytes.clone(), 0);
    }
    let stats = remote.stats.clone();
    // A fetch of a 20 000-byte file holds its buffer plus one read block.
    let per_fetch = 2 * SIZE as u64;
    let budget = TestBudget::leaked(3 * per_fetch);
    let config = Config {
        memory: Arc::new(TestMemory(budget)),
        ..Config::default()
    };
    let object = in_process(remote, config);
    let formats = Formats::register();
    assert_eq!(descriptor(&object, &formats).unwrap().len(), 8);

    assert_eq!(
        read_all(&contents(&object, &formats, 0).unwrap()).unwrap(),
        files[0]
    );
    assert!(
        stats.max_open() >= 2,
        "prefetch ran next to the requested file"
    );
    wait_until("file 1 is prefetched", || stats.finished(&name(1)));
    // Prefetch holds at most half the budget (60 000 bytes): one waiting
    // file (40 000), so file 2 starts only after Explorer takes file 1.
    std::thread::sleep(Duration::from_millis(300));
    assert!(
        !stats.opened(&name(2)),
        "prefetch stayed inside its budget share"
    );
    for (n, bytes) in files.iter().enumerate().skip(1) {
        let stream = contents(&object, &formats, n as i32).unwrap();
        assert_eq!(&read_all(&stream).unwrap(), bytes, "file {n}");
    }
    assert_eq!(stats.max_open(), 2, "the flow ceiling bounds open reads");
    assert!(budget.max_used() <= 3 * per_fetch);
    drop(object);
    wait_until("every reservation is returned", || budget.used() == 0);
}

#[test]
fn transfer_engine_task_remote_async_capability_ends_the_paste() {
    let _apartment = Apartment::enter();
    let label = "transfer_engine_task async";
    let remote = FakeRemote::new(label)
        .file("a.txt", b"eins".to_vec(), 0)
        .file("b.txt", b"zwei".to_vec(), 0);
    let object = in_process(remote, Config::default());
    let operation: IDataObjectAsyncCapability = object.cast().expect("async capability");
    assert!(unsafe { operation.GetAsyncMode() }.unwrap().as_bool());
    unsafe { operation.StartOperation(None::<&IBindCtx>) }.unwrap();
    assert!(unsafe { operation.InOperation() }.unwrap().as_bool());
    let formats = Formats::register();
    assert_eq!(descriptor(&object, &formats).unwrap().len(), 2);
    assert_eq!(
        read_all(&contents(&object, &formats, 0).unwrap()).unwrap(),
        b"eins"
    );
    // Explorer skipped b.txt (a conflict) and reports the end of its copy.
    unsafe { operation.EndOperation(S_OK, None::<&IBindCtx>, DROPEFFECT_COPY.0) }.unwrap();
    assert!(!unsafe { operation.InOperation() }.unwrap().as_bool());
    let entry = external(label);
    assert!(entry.finished);
    assert_eq!((entry.files_total, entry.files_done), (2, 1));
    assert_eq!(entry.note.as_deref(), Some("Explorer-Übergabe beendet"));
    unsafe { operation.SetAsyncMode(false) }.unwrap();
    assert!(!unsafe { operation.GetAsyncMode() }.unwrap().as_bool());
}

#[test]
#[ignore = "requires isolated remote copy/paste task runner"]
fn transfer_engine_task_remote_clipboard_ole_roundtrip() {
    assert_eq!(
        std::env::var("SMART_EXPLORER_COPY_PASTE_TASK").as_deref(),
        Ok("1"),
        "Clipboard acceptance requires the isolated, sequential remote task runner"
    );
    let _apartment = Apartment::enter();
    let label = "transfer_engine_task clipboard";
    let remote =
        FakeRemote::new(label)
            .dir("Ordner")
            .file("Ordner/datei.txt", b"Inhalt".to_vec(), MTIME_MS);
    let stats = remote.stats.clone();
    let (sequence, control) =
        set_clipboard_with(Arc::new(remote), Config::default()).expect("clipboard set");
    assert_eq!(sequence, unsafe { GetClipboardSequenceNumber() });
    assert_eq!(stats.listings(), 0, "copying lists nothing");
    let object = unsafe { OleGetClipboard() }.expect("clipboard object");
    let formats = Formats::register();
    let names: Vec<String> = descriptor(&object, &formats)
        .expect("descriptor")
        .into_iter()
        .map(|d| d.name)
        .collect();
    assert_eq!(names, ["Ordner", "Ordner\\datei.txt"]);
    assert_eq!(
        read_all(&contents(&object, &formats, 1).unwrap()).unwrap(),
        b"Inhalt"
    );
    drop(object);
    // Another owner empties the clipboard: OLE releases the object and the
    // worker ends without rendering anything.
    unsafe { OleSetClipboard(None::<&IDataObject>) }.expect("clipboard emptied");
    wait_until("the worker thread ends", || control.exited());
}
