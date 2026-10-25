//! Non-consuming lifetime guard during the synchronous remote Exec setup.
use std::{io, net::TcpStream, sync::{Arc, Condvar, Mutex}, thread, time::Duration};
use crate::share::route_cancellation::RouteCancellation;

#[cfg(windows)]
#[path = "../windows/exec_connect.rs"]
mod platform;
#[cfg(not(windows))]
#[path = "../linux_os/exec_connect.rs"]
mod platform;

pub(crate) struct SetupWatch {
    cancellation: Arc<RouteCancellation>,
    stopped: Arc<(Mutex<bool>, Condvar)>,
    worker: Option<thread::JoinHandle<()>>,
    handed_off: bool,
}

impl SetupWatch {
    pub(crate) fn start(stream: &TcpStream) -> io::Result<Self> {
        let cancellation = Arc::new(RouteCancellation::default());
        let observed = stream.try_clone()?;
        let peek_buffer = Mutex::new(Vec::new());
        cancellation.set_probe(Some(Arc::new(move || cancelled_input(&observed, &peek_buffer))))?;
        // An unsupported/failed EOF observation is a setup error before any Start.
        cancellation.check()?;
        let stopped = Arc::new((Mutex::new(false), Condvar::new()));
        let worker_stop = stopped.clone(); let worker_cancel = cancellation.clone();
        let worker = thread::Builder::new().name("daemon-exec-connect".into()).spawn(move || {
            loop {
                if worker_cancel.check().is_err() { return; }
                let (lock, wake) = &*worker_stop;
                let Ok(stop) = lock.lock() else { worker_cancel.cancel(); return; };
                if *stop { return; }
                match wake.wait_timeout(stop, Duration::from_millis(25)) {
                    Ok((stop, _)) if *stop => return,
                    Ok(_) => {},
                    Err(_) => { worker_cancel.cancel(); return; }
                }
            }
        })?;
        Ok(Self { cancellation, stopped, worker: Some(worker), handed_off: false })
    }

    pub(crate) fn cancellation(&self) -> Arc<RouteCancellation> { self.cancellation.clone() }
    pub(crate) fn check(&self) -> io::Result<()> { self.cancellation.check() }

    /// Called only after ExecReady was written; the ordinary input actor takes over.
    pub(crate) fn handoff(mut self) -> io::Result<()> {
        self.stop(); self.cancellation.check()?;
        self.cancellation.set_probe(None)?; self.handed_off = true;
        Ok(())
    }

    fn stop(&mut self) {
        let (lock, wake) = &*self.stopped;
        if let Ok(mut stopped) = lock.lock() { *stopped = true; }
        wake.notify_all();
        if let Some(worker) = self.worker.take() { let _ = worker.join(); }
    }
}

impl Drop for SetupWatch {
    fn drop(&mut self) {
        if !self.handed_off { self.cancellation.cancel(); }
        self.stop(); let _ = self.cancellation.set_probe(None);
    }
}

fn cancelled_input(stream: &TcpStream, buffer: &Mutex<Vec<u8>>) -> io::Result<bool> {
    let (closed, readable) = platform::probe(stream)?;
    if closed { return Ok(true); }
    if !readable { return Ok(false); }
    // One maximum stdin chunk, one maximum resize and four headers. Allocate only
    // when the caller actually queued input before ExecReady. Reads remain Peek.
    let mut bytes = buffer.lock().map_err(|_| io::Error::other("Exec-Aufbauprobe gesperrt"))?;
    bytes.resize(64 * 1024 + 256 + 20, 0);
    match stream.peek(&mut bytes) {
        Ok(0) => Ok(true),
        Ok(read) => {
            let mut offset = 0usize;
            while read.saturating_sub(offset) >= 5 {
                let tag = bytes[offset];
                let size = u32::from_be_bytes([bytes[offset + 1], bytes[offset + 2], bytes[offset + 3], bytes[offset + 4]]) as usize;
                if tag == 3 && size == 0 { return Ok(true); }
                if size > 64 * 1024 { return Err(io::Error::new(io::ErrorKind::InvalidData, "Exec-Eingaberahmen ist zu gross")); }
                let next = offset + 5 + size;
                if next > read { break; }
                offset = next;
            }
            Ok(false)
        },
        Err(error) if matches!(error.kind(), io::ErrorKind::WouldBlock | io::ErrorKind::Interrupted | io::ErrorKind::TimedOut) => Ok(false),
        Err(error) => Err(error),
    }
}

pub(super) fn start_remote(
    host: &super::ShareHost, target: crate::share::PeerOpenTarget, start: crate::share::ExecStart,
    retainer: Option<Arc<dyn Send + Sync>>, cancellation: Arc<RouteCancellation>, deadline: std::time::Instant,
) -> Result<super::RemoteExec, String> {
    loop {
        cancellation.check().map_err(|error| error.to_string())?;
        host.reload_now()?;
        host.drain_events();
        let (service, peer_device_id, peer_device_name) = {
            let state = host
                .state
                .lock()
                .map_err(|_| "Share-Worker gesperrt".to_string())?;
            let service = state.service.clone().ok_or_else(|| {
                "Share-Server ist nicht konfiguriert oder Auto-Connect ist aus".to_string()
            })?;
            let (device_id, device_name) = super::target_identity(&state.profiles, &target);
            (service, device_id, device_name)
        };
        service.cmd(crate::share::ShareCmd::Refresh)?;
        match service.start_exec_for_target_cancellable(&target, start.clone(), retainer.clone(), cancellation.clone(), deadline) {
            Ok(session) => {
                return Ok(super::RemoteExec {
                    view: crate::share::ExecJobView {
                        exec_id: start.exec_id.clone(),
                        peer_device_id,
                        peer_device_name,
                        program: start.display_program().to_string(),
                        command_digest: start.digest().map_err(|error| error.to_string())?,
                        state: crate::share::ExecLifecycleState::Connecting,
                        policy_revision: 0,
                        started_at: None,
                        finished_at: None,
                        terminal: None,
                    },
                    session,
                })
            }
            Err(error) if std::time::Instant::now() < deadline => {
                cancellation.check().map_err(|error| error.to_string())?;
                for _ in 0..20 {
                    cancellation.check().map_err(|error| error.to_string())?;
                    std::thread::sleep(Duration::from_millis(25));
                }
                if error.contains("authentication") {
                    return Err(error);
                }
            }
            Err(error) => return Err(error),
        }
    }
}

