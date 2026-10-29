//! G3-12 task tests: Nextcloud's chunked upload against a loopback fake that
//! keeps the parts and assembles them in name order on `MOVE .file`, like
//! Nextcloud's upload folders. The file must arrive byte for byte; plain
//! WebDAV and a server that refuses upload folders keep the single `PUT`.
use super::transfer_engine_task_tests::{propfind, Answer, Http, Seen};
use crate::vfs::{Backend, SegmentedStage};
use std::collections::{BTreeMap, HashMap};
use std::io::Write;
use std::sync::{Arc, Mutex};
use std::thread;

const ROOT: &str = "/remote.php/dav/files/alice";
const MIB: u64 = 1024 * 1024;

#[derive(Default)]
struct Cloud {
    origin: String,
    files: HashMap<String, Vec<u8>>,
    /// Upload folder → (destination it was created for, parts by name).
    uploads: HashMap<String, (String, BTreeMap<String, Vec<u8>>)>,
    refuse_uploads: bool,
    /// Every part PUT named the folder's destination and the total length.
    bad_part_headers: usize,
    /// Parts longer than this are answered 413 (a web server's body limit).
    max_part: Option<usize>,
}

fn header<'a>(seen: &'a Seen, name: &str) -> Option<&'a str> {
    seen.headers.get(name).map(String::as_str)
}

impl Cloud {
    fn answer(&mut self, seen: &Seen) -> Answer {
        let path = seen.path.trim_end_matches('/').to_string();
        let uploads = "/remote.php/dav/uploads/alice/";
        match seen.method.as_str() {
            "PROPFIND" => match self.files.get(&path) {
                Some(bytes) => propfind(&seen.path, Some(bytes.len() as u64)),
                None if path == ROOT || path.is_empty() => propfind(&seen.path, None),
                None => Answer::status(404),
            },
            "MKCOL" if path.starts_with(uploads) => {
                if self.refuse_uploads {
                    return Answer::status(405);
                }
                let destination = header(seen, "destination").unwrap_or_default().to_string();
                self.uploads.insert(path, (destination, BTreeMap::new()));
                Answer::status(201)
            }
            "PUT" if path.starts_with(uploads) => {
                let (folder, name) = path.rsplit_once('/').unwrap();
                let total = header(seen, "oc-total-length").map(str::to_string);
                let Some((destination, parts)) = self.uploads.get_mut(folder) else {
                    return Answer::status(404);
                };
                if self.max_part.is_some_and(|max| seen.body.len() > max) {
                    return Answer::status(413);
                }
                if header(seen, "destination") != Some(destination.as_str()) || total.is_none() {
                    self.bad_part_headers += 1;
                }
                parts.insert(name.to_string(), seen.body.clone());
                Answer::status(201)
            }
            "PUT" => {
                self.files.insert(path, seen.body.clone());
                Answer::status(201)
            }
            "MOVE" => self.move_to(seen, &path),
            "DELETE" => {
                self.uploads.remove(&path);
                self.files.remove(&path);
                Answer::status(204)
            }
            _ => Answer::status(500),
        }
    }

    fn move_to(&mut self, seen: &Seen, path: &str) -> Answer {
        let destination = header(seen, "destination").unwrap_or_default();
        let target = destination
            .strip_prefix(&self.origin)
            .unwrap_or(destination)
            .to_string();
        if header(seen, "overwrite") == Some("F") && self.files.contains_key(&target) {
            return Answer::status(412);
        }
        let content = match path.strip_suffix("/.file") {
            Some(folder) => {
                let Some((created_for, parts)) = self.uploads.remove(folder) else {
                    return Answer::status(404);
                };
                assert_eq!(created_for, destination, "one destination per upload");
                // Assembled in the order of the part names, like Nextcloud.
                let content: Vec<u8> = parts.into_values().flatten().collect();
                let total = header(seen, "oc-total-length").and_then(|v| v.parse().ok());
                if total != Some(content.len()) {
                    return Answer::status(400);
                }
                content
            }
            None => match self.files.remove(path) {
                Some(content) => content,
                None => return Answer::status(404),
            },
        };
        self.files.insert(target, content);
        Answer::status(201)
    }
}

fn cloud(refuse_uploads: bool) -> (Arc<Mutex<Cloud>>, Http) {
    let cloud = Arc::new(Mutex::new(Cloud {
        refuse_uploads,
        ..Cloud::default()
    }));
    let http = {
        let cloud = cloud.clone();
        Http::start(move |seen| cloud.lock().unwrap().answer(seen))
    };
    cloud.lock().unwrap().origin = http.origin();
    (cloud, http)
}

fn content(size: u64) -> Vec<u8> {
    let mut state = 0x9E37_79B9_7F4A_7C15u64;
    (0..size)
        .map(|_| {
            state ^= state << 13;
            state ^= state >> 7;
            state ^= state << 17;
            state as u8
        })
        .collect()
}

#[test]
fn speed_task_webdav_nextcloud_chunks_assemble_byte_exact() {
    let (cloud, http) = cloud(false);
    let backend = http.backend_at(ROOT);
    let size = 20 * MIB + 123;
    let data = Arc::new(content(size));
    let stage = format!("{ROOT}/gross.bin.se-upload-0123456789abcdef");
    let destination = format!("{ROOT}/gross.bin");
    let opened = backend.open_segmented_stage(&stage, size).unwrap();
    let opened: Arc<Box<dyn SegmentedStage>> = Arc::new(opened.expect("Nextcloud chunks"));
    // Three ranges as the engine cuts them, written at once, last first.
    let ranges = [
        (16 * MIB, size - 16 * MIB),
        (8 * MIB, 8 * MIB),
        (0, 8 * MIB),
    ];
    let writers: Vec<_> = ranges
        .into_iter()
        .map(|(offset, len)| {
            let (stage, data) = (opened.clone(), data.clone());
            thread::spawn(move || {
                let mut writer = stage.segment(offset, len).unwrap();
                let range = &data[offset as usize..(offset + len) as usize];
                for block in range.chunks(MIB as usize) {
                    writer.write_all(block).unwrap();
                }
                writer.flush().unwrap();
            })
        })
        .collect();
    for writer in writers {
        writer.join().unwrap();
    }
    let Ok(opened) = Arc::try_unwrap(opened) else {
        panic!("a range still holds the stage");
    };
    assert_eq!(opened.stage_path(), stage);
    opened.finish().unwrap();
    let published = crate::vfs::promote_copy_stage_evidence(&backend, &stage, &destination);
    assert!(published.is_ok(), "{published:?}");

    let cloud = cloud.lock().unwrap();
    assert_eq!(cloud.bad_part_headers, 0, "every part names its upload");
    assert!(cloud.uploads.is_empty(), "the upload folder is consumed");
    assert!(!cloud.files.contains_key(&stage), "the stage was published");
    assert!(
        cloud.files.get(&destination) == Some(&*data),
        "the file arrived byte for byte"
    );
    drop(cloud);
    let calls = http.calls();
    let parts: Vec<&str> = calls
        .iter()
        .filter(|call| call.0 == "PUT" && call.1.contains("/uploads/"))
        .map(|call| call.1.rsplit('/').next().unwrap())
        .collect();
    let mut sorted = parts.clone();
    sorted.sort_unstable();
    // Offsets 0, 8 MiB and 16 MiB in 5 MiB units: parts 1, 2 and 4.
    assert_eq!(sorted, ["00001", "00002", "00004"]);
    // Published right after the checked assembly: no PROPFIND before the MOVE.
    let last = &calls[calls.len() - 1];
    assert_eq!((last.0.as_str(), last.1.as_str()), ("MOVE", stage.as_str()));
    assert_eq!(calls[calls.len() - 2].0, "PROPFIND");
}

#[test]
fn speed_task_webdav_plain_server_keeps_the_single_put() {
    let (_cloud, http) = cloud(false);
    let backend = http.backend_at("/");
    assert!(backend
        .open_segmented_stage("/dav/a.bin.se-upload-0123456789abcdef", 20 * MIB)
        .unwrap()
        .is_none());
    assert!(http.calls().iter().all(|(method, _)| method != "MKCOL"));
}

#[test]
fn speed_task_webdav_refused_upload_folder_is_remembered() {
    let (_cloud, http) = cloud(true);
    let backend = http.backend_at(ROOT);
    let stage = format!("{ROOT}/a.bin.se-upload-0123456789abcdef");
    for _ in 0..2 {
        assert!(backend
            .open_segmented_stage(&stage, 20 * MIB)
            .unwrap()
            .is_none());
    }
    let asked = http
        .calls()
        .iter()
        .filter(|(method, _)| method == "MKCOL")
        .count();
    assert_eq!(asked, 1, "the server is asked once per connection");
}

#[test]
fn speed_task_webdav_chunk_abort_removes_the_upload_folder() {
    let (cloud, http) = cloud(false);
    let backend = http.backend_at(ROOT);
    let stage = format!("{ROOT}/b.bin.se-upload-0123456789abcdef");
    let opened = backend.open_segmented_stage(&stage, 20 * MIB).unwrap();
    let opened = opened.expect("Nextcloud chunks");
    let mut writer = opened.segment(0, 8 * MIB).unwrap();
    writer.write_all(&vec![1u8; 8 * MIB as usize]).unwrap();
    writer.flush().unwrap();
    drop(writer);
    // A range shorter than a part that does not end the file is refused.
    assert!(opened.segment(8 * MIB, MIB).is_err());
    // Not every part is there: finish refuses and nothing is assembled.
    let incomplete = backend.open_segmented_stage(&stage, 20 * MIB).unwrap();
    let incomplete = incomplete.expect("Nextcloud chunks");
    assert!(incomplete.finish().is_err());
    opened.abort();
    let cloud = cloud.lock().unwrap();
    assert!(cloud.uploads.is_empty(), "both folders are gone");
    assert!(!cloud.files.contains_key(&stage));
}

#[test]
fn speed_task_webdav_part_too_large_starts_again_with_smaller_parts() {
    let (cloud, http) = cloud(false);
    cloud.lock().unwrap().max_part = Some(16 * MIB as usize);
    let backend = http.backend_at(ROOT);
    let size = 30 * MIB;
    let data = content(size);
    let stage = format!("{ROOT}/c.bin.se-upload-0123456789abcdef");
    let first = backend.open_segmented_stage(&stage, size).unwrap();
    let first = first.expect("Nextcloud chunks");
    let mut writer = first.segment(0, size).unwrap();
    let written = data
        .chunks(MIB as usize)
        .try_for_each(|block| writer.write_all(block))
        .and_then(|()| writer.flush());
    let error = written.expect_err("one part of 30 MiB is refused");
    assert!(
        crate::vfs::congestion_of(&error).is_some(),
        "not job-ending: {error}"
    );
    drop(writer);
    first.abort();
    // The file starts again: the server's cap is now 15 MiB.
    let second = backend.open_segmented_stage(&stage, size).unwrap();
    let second = second.expect("Nextcloud chunks");
    let mut writer = second.segment(0, size).unwrap();
    for block in data.chunks(MIB as usize) {
        writer.write_all(block).unwrap();
    }
    writer.flush().unwrap();
    drop(writer);
    second.finish().unwrap();
    let cloud = cloud.lock().unwrap();
    assert!(cloud.files.get(&stage) == Some(&data), "byte for byte");
}
