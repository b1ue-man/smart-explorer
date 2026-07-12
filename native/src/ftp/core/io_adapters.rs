use std::io::{self, Read};
use std::sync::{Arc, Condvar, Mutex, MutexGuard, Weak};
use std::time::{Duration, Instant};

use suppaftp::RustlsFtpStream;

fn io_err<E: std::fmt::Display>(error: E) -> io::Error {
    io::Error::other(error.to_string())
}

/// Owns the single FTP control connection. A streaming RETR checks the
/// connection out until its data stream is finalized or aborted; other calls
/// wait rather than issuing commands into an active transfer's response.
pub(super) struct FtpConnection {
    state: Mutex<ControlState>,
    available: Condvar,
    reconnect: FtpReconnect,
    keepalive: Arc<KeepaliveControl>,
}

struct ControlState {
    stream: Option<RustlsFtpStream>,
    last_activity: Instant,
    health: ControlHealth,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ControlHealth {
    Healthy,
    Suspect,
}

struct KeepaliveControl {
    stopped: Mutex<bool>,
    wake: Condvar,
}

pub(super) type FtpReconnect = Arc<dyn Fn() -> io::Result<RustlsFtpStream> + Send + Sync>;

const FTP_KEEPALIVE_INTERVAL: Duration = Duration::from_secs(15);
const FTP_KEEPALIVE_IO_TIMEOUT: Duration = Duration::from_secs(10);

impl FtpConnection {
    pub(super) fn new(stream: RustlsFtpStream, reconnect: FtpReconnect) -> io::Result<Arc<Self>> {
        Self::new_with_timing(
            stream,
            reconnect,
            FTP_KEEPALIVE_INTERVAL,
            FTP_KEEPALIVE_IO_TIMEOUT,
        )
    }

    pub(super) fn new_with_timing(
        stream: RustlsFtpStream,
        reconnect: FtpReconnect,
        interval: Duration,
        io_timeout: Duration,
    ) -> io::Result<Arc<Self>> {
        let connection = Arc::new(Self {
            state: Mutex::new(ControlState {
                stream: Some(stream),
                last_activity: Instant::now(),
                health: ControlHealth::Healthy,
            }),
            available: Condvar::new(),
            reconnect,
            keepalive: Arc::new(KeepaliveControl {
                stopped: Mutex::new(false),
                wake: Condvar::new(),
            }),
        });
        Self::spawn_keepalive(&connection, interval, io_timeout)?;
        Ok(connection)
    }

    fn wait_for_stream(&self) -> io::Result<MutexGuard<'_, ControlState>> {
        let mut state = self
            .state
            .lock()
            .map_err(|_| io_err("FTP-Verbindung vergiftet"))?;
        while state.stream.is_none() {
            state = self
                .available
                .wait(state)
                .map_err(|_| io_err("FTP-Verbindung vergiftet"))?;
        }
        Ok(state)
    }

    fn reconnect_locked(&self, state: &mut ControlState) -> io::Result<()> {
        let replacement = (self.reconnect)()?;
        state.stream = Some(replacement);
        state.health = ControlHealth::Healthy;
        state.last_activity = Instant::now();
        Ok(())
    }

    fn ensure_healthy(&self, state: &mut ControlState) -> io::Result<()> {
        if state.health == ControlHealth::Suspect {
            self.reconnect_locked(state)?;
        }
        Ok(())
    }

    pub(super) fn with_stream_mutation<T>(
        &self,
        operation: impl FnOnce(&mut RustlsFtpStream) -> io::Result<T>,
    ) -> io::Result<T> {
        let mut state = self.wait_for_stream()?;
        self.ensure_healthy(&mut state)?;
        let result = operation(
            state
                .stream
                .as_mut()
                .ok_or_else(|| io_err("FTP-Verbindung ist nicht verfügbar"))?,
        );
        state.last_activity = Instant::now();
        if result.is_err() {
            state.health = ControlHealth::Suspect;
        }
        drop(state);
        self.keepalive.wake.notify_all();
        result
    }

    pub(super) fn with_stream_read<T>(
        &self,
        mut operation: impl FnMut(&mut RustlsFtpStream) -> io::Result<T>,
    ) -> io::Result<T> {
        let mut state = self.wait_for_stream()?;
        self.ensure_healthy(&mut state)?;
        let first = operation(
            state
                .stream
                .as_mut()
                .ok_or_else(|| io_err("FTP-Verbindung ist nicht verfügbar"))?,
        );
        let result = match first {
            Ok(value) => Ok(value),
            Err(first_error) => {
                state.health = ControlHealth::Suspect;
                match self.reconnect_locked(&mut state) {
                    Ok(()) => operation(
                        state
                            .stream
                            .as_mut()
                            .ok_or_else(|| io_err("FTP-Verbindung ist nicht verfügbar"))?,
                    ),
                    Err(reconnect_error) => Err(io::Error::new(
                        first_error.kind(),
                        format!(
                            "{first_error}; FTP-Wiederverbindung vor sicherem Lese-Retry fehlgeschlagen: {reconnect_error}"
                        ),
                    )),
                }
            }
        };
        state.last_activity = Instant::now();
        state.health = if result.is_ok() {
            ControlHealth::Healthy
        } else {
            ControlHealth::Suspect
        };
        drop(state);
        self.keepalive.wake.notify_all();
        result
    }

    pub(super) fn open_reader(self: &Arc<Self>, path: &str) -> io::Result<FtpReader> {
        let mut state = self.wait_for_stream()?;
        self.ensure_healthy(&mut state)?;
        let data = match state
            .stream
            .as_mut()
            .ok_or_else(|| io_err("FTP-Verbindung ist nicht verfügbar"))?
            .retr_as_stream(path)
            .map_err(io_err)
        {
            Ok(data) => data,
            Err(first_error) => {
                state.health = ControlHealth::Suspect;
                self.reconnect_locked(&mut state).map_err(|reconnect_error| {
                    io::Error::new(
                        first_error.kind(),
                        format!(
                            "{first_error}; FTP-Wiederverbindung vor RETR fehlgeschlagen: {reconnect_error}"
                        ),
                    )
                })?;
                match state
                    .stream
                    .as_mut()
                    .ok_or_else(|| io_err("FTP-Verbindung ist nicht verfügbar"))?
                    .retr_as_stream(path)
                    .map_err(io_err)
                {
                    Ok(data) => data,
                    Err(error) => {
                        state.health = ControlHealth::Suspect;
                        state.last_activity = Instant::now();
                        drop(state);
                        self.keepalive.wake.notify_all();
                        return Err(error);
                    }
                }
            }
        };
        let control = state
            .stream
            .take()
            .ok_or_else(|| io_err("FTP-Verbindung ist nicht verfügbar"))?;
        drop(state);
        Ok(FtpReader {
            owner: self.clone(),
            control: Some(control),
            data: Some(Box::new(data)),
        })
    }

    fn return_stream(&self, stream: RustlsFtpStream, healthy: bool) {
        let mut state = match self.state.lock() {
            Ok(state) => state,
            Err(poisoned) => poisoned.into_inner(),
        };
        debug_assert!(
            state.stream.is_none(),
            "FTP control connection returned twice"
        );
        if state.stream.is_none() {
            state.stream = Some(stream);
            state.last_activity = Instant::now();
            state.health = if healthy {
                ControlHealth::Healthy
            } else {
                ControlHealth::Suspect
            };
        }
        drop(state);
        self.available.notify_one();
        self.keepalive.wake.notify_all();
    }

    fn spawn_keepalive(
        connection: &Arc<Self>,
        interval: Duration,
        io_timeout: Duration,
    ) -> io::Result<()> {
        let weak = Arc::downgrade(connection);
        let control = connection.keepalive.clone();
        std::thread::Builder::new()
            .name("ftp-keepalive".to_string())
            .spawn(move || run_keepalive(weak, control, interval, io_timeout))?;
        Ok(())
    }

    fn keepalive_once(&self, interval: Duration, io_timeout: Duration) {
        let Ok(mut state) = self.state.try_lock() else {
            return;
        };
        if state.health == ControlHealth::Healthy && state.last_activity.elapsed() < interval {
            return;
        }
        if state.health == ControlHealth::Suspect {
            if self.reconnect_locked(&mut state).is_err() {
                state.last_activity = Instant::now();
            }
            return;
        }
        let Some(stream) = state.stream.as_mut() else {
            return;
        };
        let previous_read_timeout = stream.get_ref().read_timeout().ok().flatten();
        let previous_write_timeout = stream.get_ref().write_timeout().ok().flatten();
        let configured = stream
            .get_ref()
            .set_read_timeout(Some(io_timeout))
            .and_then(|()| stream.get_ref().set_write_timeout(Some(io_timeout)));
        let ping = configured.and_then(|()| stream.noop().map_err(io_err));
        let _ = stream.get_ref().set_read_timeout(previous_read_timeout);
        let _ = stream.get_ref().set_write_timeout(previous_write_timeout);
        if ping.is_err() {
            state.health = ControlHealth::Suspect;
            if self.reconnect_locked(&mut state).is_err() {
                state.last_activity = Instant::now();
            }
            return;
        }
        state.health = ControlHealth::Healthy;
        state.last_activity = Instant::now();
    }

    fn keepalive_delay(&self, interval: Duration) -> Duration {
        let Ok(state) = self.state.lock() else {
            return Duration::ZERO;
        };
        if state.stream.is_none() {
            return interval;
        }
        interval.saturating_sub(state.last_activity.elapsed())
    }
}

fn run_keepalive(
    connection: Weak<FtpConnection>,
    control: Arc<KeepaliveControl>,
    interval: Duration,
    io_timeout: Duration,
) {
    loop {
        let delay = match connection.upgrade() {
            Some(connection) => connection.keepalive_delay(interval),
            None => return,
        };
        let Ok(stopped) = control.stopped.lock() else {
            return;
        };
        let Ok((stopped, wait)) = control.wake.wait_timeout(stopped, delay) else {
            return;
        };
        if *stopped {
            return;
        }
        drop(stopped);
        if !wait.timed_out() {
            continue;
        }
        let Some(connection) = connection.upgrade() else {
            return;
        };
        connection.keepalive_once(interval, io_timeout);
    }
}

impl Drop for FtpConnection {
    fn drop(&mut self) {
        if let Ok(mut stopped) = self.keepalive.stopped.lock() {
            *stopped = true;
            self.keepalive.wake.notify_all();
        }
    }
}

pub(super) struct FtpReader {
    owner: Arc<FtpConnection>,
    control: Option<RustlsFtpStream>,
    data: Option<Box<dyn Read + Send>>,
}

impl FtpReader {
    fn close(&mut self, completed: bool) -> io::Result<()> {
        let Some(mut control) = self.control.take() else {
            return Ok(());
        };
        let result = match self.data.take() {
            Some(data) if completed => control.finalize_retr_stream(data).map_err(io_err),
            Some(data) => control.abort(data).map_err(io_err),
            None => Err(io_err("FTP-Datenstrom fehlt")),
        };
        self.owner.return_stream(control, result.is_ok());
        result
    }
}

impl Read for FtpReader {
    fn read(&mut self, buffer: &mut [u8]) -> io::Result<usize> {
        if buffer.is_empty() {
            return Ok(0);
        }
        let Some(data) = self.data.as_mut() else {
            return Ok(0);
        };
        match data.read(buffer) {
            Ok(0) => self.close(true).map(|()| 0),
            Ok(read) => Ok(read),
            Err(error) => {
                let _ = self.close(false);
                Err(error)
            }
        }
    }
}

impl Drop for FtpReader {
    fn drop(&mut self) {
        if self.data.is_some() {
            let _ = self.close(false);
        }
    }
}
