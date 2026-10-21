//! The background service in front of a backend that writes ranges: frames
//! of `range-v1` reach the backend's stage, the labels follow the backend,
//! and an unfinished stage goes with its session.
use std::fs::File;
use std::io::{self, Read, Write};
use std::net::{Shutdown, TcpListener, TcpStream};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use super::super::serve_backend;
use crate::agent::AgentBackend;
use crate::agent_proto::{read_frame, write_frame, Frame};
use crate::vfs::{
    Backend, BackendHandle, LocalBackend, Scheme, SegmentedStage, VfsMeta, VfsResult,
};

const MIB: usize = 1024 * 1024;

/// A local backend that offers a stage for ranges (or, `ranges` false, not
/// even range reads), and writes down the stages it was asked for.
struct RangeBackend {
    inner: LocalBackend,
    ranges: bool,
    aborted: Arc<Mutex<Vec<String>>>,
    hints: Arc<Mutex<Vec<String>>>,
}

struct FileStage {
    file: Arc<File>,
    path: PathBuf,
    name: String,
    size: u64,
    aborted: Arc<Mutex<Vec<String>>>,
}

struct RangeWriter {
    file: Arc<File>,
    at: u64,
    left: u64,
}

impl Write for RangeWriter {
    fn write(&mut self, data: &[u8]) -> io::Result<usize> {
        if data.len() as u64 > self.left {
            return Err(io::Error::from(io::ErrorKind::InvalidData));
        }
        crate::transfer::write_all_at(&self.file, data, self.at)?;
        self.at += data.len() as u64;
        self.left -= data.len() as u64;
        Ok(data.len())
    }
    fn flush(&mut self) -> io::Result<()> {
        if self.left == 0 {
            Ok(())
        } else {
            Err(io::Error::from(io::ErrorKind::InvalidData))
        }
    }
}

impl SegmentedStage for FileStage {
    fn stage_path(&self) -> &str {
        &self.name
    }
    fn segment(&self, offset: u64, len: u64) -> VfsResult<Box<dyn Write + Send>> {
        Ok(Box::new(RangeWriter {
            file: self.file.clone(),
            at: offset,
            left: len,
        }))
    }
    fn finish(self: Box<Self>) -> VfsResult<()> {
        if self.file.metadata()?.len() == self.size {
            Ok(())
        } else {
            Err(io::Error::from(io::ErrorKind::InvalidData))
        }
    }
    fn abort(self: Box<Self>) {
        let _ = std::fs::remove_file(&self.path);
        self.aborted.lock().unwrap().push(self.name.clone());
    }
}

impl Backend for RangeBackend {
    fn scheme(&self) -> Scheme {
        self.inner.scheme()
    }
    fn root_display(&self) -> String {
        self.inner.root_display()
    }
    fn list_dir(&self, path: &str) -> VfsResult<Vec<VfsMeta>> {
        self.inner.list_dir(path)
    }
    fn stat(&self, path: &str) -> VfsResult<VfsMeta> {
        self.inner.stat(path)
    }
    fn open_read(&self, path: &str) -> VfsResult<Box<dyn Read + Send>> {
        self.inner.open_read(path)
    }
    fn open_write(&self, path: &str) -> VfsResult<Box<dyn Write + Send>> {
        self.inner.open_write(path)
    }
    fn rename(&self, src: &str, dst: &str) -> VfsResult<()> {
        self.inner.rename(src, dst)
    }
    fn remove_file(&self, path: &str) -> VfsResult<()> {
        self.inner.remove_file(path)
    }
    fn remove_dir(&self, path: &str) -> VfsResult<()> {
        self.inner.remove_dir(path)
    }
    fn mkdir_all(&self, path: &str) -> VfsResult<()> {
        self.inner.mkdir_all(path)
    }
    fn promote_staged_no_replace(&self, staged: &str, destination: &str) -> VfsResult<()> {
        self.inner.promote_staged_no_replace(staged, destination)
    }
    fn supports_range_reads(&self, _path: &str) -> bool {
        self.ranges
    }
    fn open_segmented_stage(
        &self,
        stage: &str,
        size: u64,
    ) -> VfsResult<Option<Box<dyn SegmentedStage>>> {
        if !self.ranges {
            return Ok(None);
        }
        self.hints.lock().unwrap().push(stage.to_string());
        let path = PathBuf::from(stage);
        let file = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&path)?;
        file.set_len(size)?;
        Ok(Some(Box::new(FileStage {
            file: Arc::new(file),
            path,
            name: stage.to_string(),
            size,
            aborted: self.aborted.clone(),
        })))
    }
}

struct Served {
    address: std::net::SocketAddr,
    server: std::thread::JoinHandle<()>,
    aborted: Arc<Mutex<Vec<String>>>,
    hints: Arc<Mutex<Vec<String>>>,
}

fn serve(backend_ranges: Option<bool>) -> Served {
    let aborted = Arc::new(Mutex::new(Vec::new()));
    let hints = Arc::new(Mutex::new(Vec::new()));
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap();
    let (a, h) = (aborted.clone(), hints.clone());
    let server = std::thread::spawn(move || {
        let (socket, _) = listener.accept().unwrap();
        let read = socket.try_clone().unwrap();
        let backend: BackendHandle = match backend_ranges {
            Some(ranges) => Arc::new(RangeBackend {
                inner: LocalBackend::new("/"),
                ranges,
                aborted: a,
                hints: h,
            }),
            None => Arc::new(LocalBackend::new("/")),
        };
        let _ = serve_backend(read, socket, backend);
    });
    Served {
        address,
        server,
        aborted,
        hints,
    }
}

fn client(address: std::net::SocketAddr) -> (AgentBackend, TcpStream) {
    let socket = TcpStream::connect(address).unwrap();
    let shutdown = socket.try_clone().unwrap();
    let backend = AgentBackend::from_streams(
        Box::new(socket.try_clone().unwrap()) as Box<dyn Read + Send>,
        Box::new(socket) as Box<dyn Write + Send>,
        Arc::new(LocalBackend::new("/")),
    )
    .unwrap();
    (backend, shutdown)
}

#[test]
fn speed_task_service_forwards_ranges_to_the_backend_stage() {
    let dir = tempfile::tempdir().unwrap();
    let destination = dir.path().join("big.bin").to_string_lossy().into_owned();
    let served = serve(Some(true));
    let (backend, shutdown) = client(served.address);
    let size = 20 * MIB + 77;
    let body: Vec<u8> = (0..size).map(|index| (index % 249) as u8).collect();

    let hint = format!("{destination}.se-upload-0123456789abcdef");
    let stage = backend
        .open_segmented_stage(&hint, size as u64)
        .unwrap()
        .expect("the service announced range-v1 and forwards the stage");
    let name = stage.stage_path().to_string();
    assert_ne!(name, hint, "the service picks the name, not the client");
    assert_eq!(served.hints.lock().unwrap().as_slice(), [name.clone()]);
    std::thread::scope(|scope| {
        let (stage, body) = (&*stage, &body);
        for (from, to) in [(0, 8 * MIB), (8 * MIB, 16 * MIB), (16 * MIB, size)]
            .into_iter()
            .rev()
        {
            scope.spawn(move || {
                let mut writer = stage.segment(from as u64, (to - from) as u64).unwrap();
                writer.write_all(&body[from..to]).unwrap();
                writer.flush().unwrap();
            });
        }
    });
    stage.finish().unwrap();
    backend
        .promote_staged_no_replace(&name, &destination)
        .unwrap();
    assert_eq!(std::fs::read(&destination).unwrap(), body);

    let _ = shutdown.shutdown(Shutdown::Both);
    drop(backend);
    served.server.join().unwrap();

    // Raw frames: an overlap is refused, and an unfinished stage is aborted
    // with its session.
    let raw = serve(Some(true));
    let mut writer = TcpStream::connect(raw.address).unwrap();
    let mut reader = writer.try_clone().unwrap();
    let second = dir.path().join("second.bin").to_string_lossy().into_owned();
    let hello = Frame::Hello {
        proto: crate::agent_proto::PROTO_VERSION,
    };
    write_frame(&mut writer, 1, &hello).unwrap();
    let _ = read_frame(&mut reader).unwrap();
    let open = Frame::SegStageOpen {
        path: second,
        size: 1000,
    };
    write_frame(&mut writer, 2, &open).unwrap();
    let Some((_, Frame::SegStageReady { stage })) = read_frame(&mut reader).unwrap() else {
        panic!("a stage was expected");
    };
    let at = |offset, len| Frame::WriteAt {
        stage: stage.clone(),
        offset,
        len,
    };
    write_frame(&mut writer, 3, &at(0, 600)).unwrap();
    assert!(matches!(
        read_frame(&mut reader).unwrap(),
        Some((3, Frame::Progress { .. }))
    ));
    write_frame(&mut writer, 4, &at(500, 100)).unwrap();
    assert!(
        matches!(read_frame(&mut reader).unwrap(), Some((4, Frame::Err(_)))),
        "a taken range"
    );
    write_frame(&mut writer, 5, &at(900, 200)).unwrap();
    assert!(
        matches!(read_frame(&mut reader).unwrap(), Some((5, Frame::Err(_)))),
        "outside the stage"
    );
    let _ = writer.shutdown(Shutdown::Both);
    raw.server.join().unwrap();
    let aborted = raw.aborted.lock().unwrap();
    assert!(
        aborted.iter().any(|name| name.contains("second.bin")),
        "{aborted:?}"
    );
}

#[test]
fn speed_task_service_without_backend_support_announces_no_labels() {
    let dir = tempfile::tempdir().unwrap();
    // (backend, labels announced, stage offered by the backend)
    for (backend_ranges, labels, stage_offered) in [
        (Some(false), false, false),
        (None, true, false),
        (Some(true), true, true),
    ] {
        let served = serve(backend_ranges);
        let (backend, shutdown) = client(served.address);
        let version = backend.version().to_string();
        assert_eq!(version.contains("range-v1"), labels, "{version}");
        assert_eq!(
            version.contains("lz4-v1"),
            labels && !crate::agent_proto::lz4_switched_off(),
            "{version}"
        );
        let hint = format!(
            "{}/never.bin.se-upload-0123456789abcdef",
            dir.path().to_string_lossy()
        );
        let stage = backend.open_segmented_stage(&hint, 100).unwrap();
        assert_eq!(stage.is_some(), stage_offered, "{backend_ranges:?}");
        if let Some(stage) = stage {
            stage.abort();
        } else {
            assert!(served.hints.lock().unwrap().is_empty());
        }
        let _ = shutdown.shutdown(Shutdown::Both);
        drop(backend);
        served.server.join().unwrap();
    }
}
