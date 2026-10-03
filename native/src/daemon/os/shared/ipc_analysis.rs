//! Dedicated authenticated GUI/worker analysis stream; ordinary file I/O stays
//! on the existing agent connection. A closed stream cancels the actual scan.
use crate::analytics::{
    analysis_transfer::{self, AnalysisMessage, AnalysisReceiver},
    Progress, ScanOutcome, ScanPhase,
};
use crate::vfs::BackendHandle;
use std::{
    io::{self, Read, Write},
    net::{Shutdown, TcpStream},
    sync::atomic::{AtomicBool, Ordering},
    sync::{Arc, Mutex},
    thread,
    time::{Duration, Instant},
};

const CONTROL: u8 = 0;
const DATA: u8 = 1;
const ERROR: u8 = 2;
const MAX_FRAME: usize = 1024 * 1024;

pub(super) fn scan(
    target: crate::share::PeerOpenTarget,
    root: &str,
    progress: &Progress,
) -> io::Result<ScanOutcome> {
    progress.check_cancel()?;
    // Freeze the offered limit: request and decoder must use the same value.
    progress.set_node_budget(progress.node_budget());
    let token = super::ipc_storage::read_token().map_err(io::Error::other)?;
    let address = super::ipc_storage::read_ipc_addr()
        .ok_or_else(|| io::Error::other("Analyse-Worker nicht erreichbar"))?;
    let mut stream = TcpStream::connect_timeout(&address, Duration::from_secs(3))?;
    stream.set_write_timeout(Some(Duration::from_secs(60)))?;
    stream.set_nodelay(true)?;
    super::ipc_protocol::write_request(
        &mut stream,
        &super::ipc_protocol::IpcRequest::AnalyzeShare {
            token,
            target,
            root: root.into(),
            node_budget: Some(progress.node_budget()),
        },
    )?;
    receive(stream, progress)
}

struct ReaderGuard {
    socket: TcpStream,
    reader: Option<thread::JoinHandle<()>>,
}
impl Drop for ReaderGuard {
    fn drop(&mut self) {
        let _ = self.socket.shutdown(Shutdown::Both);
        if let Some(reader) = self.reader.take() {
            let _ = reader.join();
        }
    }
}

pub(super) fn receive(stream: TcpStream, progress: &Progress) -> io::Result<ScanOutcome> {
    let mut read = stream.try_clone()?;
    read.set_read_timeout(Some(Duration::from_secs(60)))?;
    let (send, frames) = std::sync::mpsc::sync_channel(2);
    let reader = thread::Builder::new()
        .name("analysis-ipc-reader".into())
        .spawn(move || loop {
            let frame = read_frame(&mut read);
            let failed = frame.is_err();
            if send.send(frame).is_err() || failed {
                break;
            }
        })?;
    let guard = ReaderGuard {
        socket: stream,
        reader: Some(reader),
    };
    let mut receiver = AnalysisReceiver::with_node_budget(progress.node_budget());
    let mut last = Instant::now();
    let result = (|| loop {
        progress.check_cancel()?;
        let (tag, bytes) = match frames.recv_timeout(Duration::from_millis(100)) {
            Ok(frame) => {
                last = Instant::now();
                frame?
            }
            Err(std::sync::mpsc::RecvTimeoutError::Timeout)
                if last.elapsed() < Duration::from_secs(60) =>
            {
                continue
            }
            Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {
                return Err(io::Error::new(
                    io::ErrorKind::TimedOut,
                    "Analyse-Worker meldet seit 60 Sekunden keinen Zustand",
                ))
            }
            Err(_) => {
                return Err(io::Error::new(
                    io::ErrorKind::UnexpectedEof,
                    "Analyse-Worker wurde ohne Ergebnis beendet",
                ))
            }
        };
        match tag {
            CONTROL => {
                let message = serde_json::from_slice(&bytes).map_err(io::Error::other)?;
                if let Some(outcome) = receiver.control(message, progress)? {
                    return Ok(outcome);
                }
            }
            DATA => receiver.data(&bytes, progress)?,
            ERROR => {
                return Err(io::Error::other(
                    String::from_utf8_lossy(&bytes).into_owned(),
                ))
            }
            _ => {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    "Unbekannter Analyse-IPC-Frame",
                ))
            }
        }
    })();
    // Drop the receiver before joining: a full bounded channel must also wake.
    drop(frames);
    drop(guard);
    result
}

pub(super) fn serve(
    stream: TcpStream,
    root: String,
    node_budget: Option<u64>,
    open: impl FnOnce() -> io::Result<BackendHandle>,
) -> io::Result<()> {
    stream.set_write_timeout(Some(Duration::from_secs(60)))?;
    stream.set_nodelay(true)?;
    let mut read = stream.try_clone()?;
    let sink = Arc::new(Mutex::new(stream.try_clone()?));
    let progress = Progress::default();
    if let Some(budget) = node_budget {
        progress.set_node_budget(budget);
    }
    progress.set_phase(ScanPhase::Preparing, &root);
    let cancellation = progress.cancel.clone();
    let reader = thread::Builder::new()
        .name("analysis-ipc-cancel".into())
        .spawn(move || {
            let mut byte = [0];
            let _ = read.read(&mut byte);
            cancellation.store(true, Ordering::Relaxed);
        })?;
    let _reader = ReaderGuard {
        socket: stream,
        reader: Some(reader),
    };
    let done = Arc::new(AtomicBool::new(false));
    let stop = done.clone();
    let live = progress.clone();
    let writer = sink.clone();
    let emitter = thread::Builder::new()
        .name("analysis-ipc-progress".into())
        .spawn(move || {
            while !stop.load(Ordering::Relaxed) {
                if send_control(
                    &writer,
                    AnalysisMessage::Progress {
                        state: live.snapshot(),
                    },
                )
                .is_err()
                {
                    live.cancel.store(true, Ordering::Relaxed);
                    break;
                }
                thread::park_timeout(Duration::from_millis(250));
            }
        })?;
    let scanned = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let backend = open()?;
        Ok::<_, io::Error>(crate::analytics::scan_remote(&*backend, &root, &progress))
    }))
    .unwrap_or_else(|_| Err(io::Error::other("Analyse-Worker unerwartet beendet")));
    done.store(true, Ordering::Relaxed);
    emitter.thread().unpark();
    let emitter_result = emitter
        .join()
        .map_err(|_| io::Error::other("Analyse-Fortschrittskanal beendet"));
    let result = scanned.and_then(|mut outcome| {
        emitter_result?;
        progress.check_cancel()?;
        progress.set_phase(ScanPhase::Assembling, &root);
        analysis_transfer::send_outcome_with(
            &mut outcome,
            &progress,
            analysis_transfer::SendOptions {
                deflate: true,
                host_scan_ms: None,
            },
            |message| send_control(&sink, message),
            |bytes| send_frame(&sink, DATA, &bytes),
        )
    });
    if let Err(error) = &result {
        let _ = send_frame(&sink, ERROR, error.to_string().as_bytes());
    }
    result
}

fn send_control(sink: &Mutex<TcpStream>, message: AnalysisMessage) -> io::Result<()> {
    send_frame(
        sink,
        CONTROL,
        &serde_json::to_vec(&message).map_err(io::Error::other)?,
    )
}

fn send_frame(sink: &Mutex<TcpStream>, tag: u8, bytes: &[u8]) -> io::Result<()> {
    if bytes.len() >= MAX_FRAME {
        return Err(io::Error::other("Analyse-IPC-Frame zu groß"));
    }
    let mut stream = sink
        .lock()
        .map_err(|_| io::Error::other("Analyse-IPC gesperrt"))?;
    stream.write_all(&((bytes.len() + 1) as u32).to_be_bytes())?;
    stream.write_all(&[tag])?;
    stream.write_all(bytes)
}

fn read_frame(stream: &mut TcpStream) -> io::Result<(u8, Vec<u8>)> {
    let mut header = [0; 4];
    stream.read_exact(&mut header)?;
    let length = u32::from_be_bytes(header) as usize;
    if length == 0 || length > MAX_FRAME {
        return Err(io::Error::other("Ungültiger Analyse-IPC-Frame"));
    }
    let mut bytes = vec![0; length];
    stream.read_exact(&mut bytes)?;
    let tag = bytes.remove(0);
    Ok((tag, bytes))
}
