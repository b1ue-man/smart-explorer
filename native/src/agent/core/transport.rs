use super::mux::{close_transport, make_out_channel, route_frame, Mux};
use crate::agent_proto::{self, Frame};
use std::collections::HashMap;
use std::io::{self, Read, Write};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

pub(super) type AgentStreams = (Box<dyn Read + Send>, Box<dyn Write + Send>);
pub(super) type AgentReconnect = Arc<dyn Fn() -> io::Result<AgentStreams> + Send + Sync>;

#[derive(Clone, Copy)]
pub(super) struct HeartbeatPolicy {
    idle: Duration,
    deadline: Duration,
}

impl HeartbeatPolicy {
    pub(super) const fn new(idle: Duration, deadline: Duration) -> Self {
        Self { idle, deadline }
    }
}

impl Default for HeartbeatPolicy {
    fn default() -> Self {
        Self::new(Duration::from_secs(30), Duration::from_secs(30))
    }
}

struct TransportGeneration {
    mux: Arc<Mux>,
}

struct TransportState {
    current: TransportGeneration,
    next_id: u64,
}

pub(super) struct AgentConnection {
    state: Mutex<TransportState>,
    reconnect: Option<AgentReconnect>,
    heartbeat: HeartbeatPolicy,
}

impl AgentConnection {
    pub(super) fn new(
        streams: AgentStreams,
        reconnect: Option<AgentReconnect>,
    ) -> io::Result<(Arc<Self>, String)> {
        Self::new_with_heartbeat(streams, reconnect, HeartbeatPolicy::default())
    }

    pub(super) fn new_with_heartbeat(
        streams: AgentStreams,
        reconnect: Option<AgentReconnect>,
        heartbeat: HeartbeatPolicy,
    ) -> io::Result<(Arc<Self>, String)> {
        let (mux, version) = establish(streams, heartbeat.deadline)?;
        let connection = Arc::new(Self {
            state: Mutex::new(TransportState {
                current: TransportGeneration { mux: mux.clone() },
                next_id: 2,
            }),
            reconnect,
            heartbeat,
        });
        connection.start_heartbeat(mux, 1)?;
        Ok((connection, version))
    }

    /// Return a live mux, replacing a generation only before this caller has
    /// dispatched an operation on it.
    pub(super) fn mux(self: &Arc<Self>) -> io::Result<Arc<Mux>> {
        let current = self.current_mux();
        if !current.is_closed() && !current.is_retired() {
            return Ok(current);
        }
        self.replace_unusable(&current)
    }

    /// Retry an idempotent one-request/one-response read at most once. A timed
    /// out reconnectable generation stops accepting new requests but drains
    /// its registered mutation streams while the retry uses a replacement.
    /// A reconnect-less local proxy only unregisters the timed-out request.
    pub(super) fn safe_call_timeout(
        self: &Arc<Self>,
        request: Frame,
        timeout: Duration,
    ) -> io::Result<Frame> {
        let first = self.mux()?;
        match self.metadata_attempt(&first, request.clone(), timeout) {
            Ok(frame) => Ok(frame),
            Err(error) if error.kind() == io::ErrorKind::TimedOut && self.reconnect.is_some() => {
                let replacement = self.replace_unusable(&first)?;
                self.metadata_attempt(&replacement, request, timeout)
            }
            Err(_error) if first.is_closed() || first.is_retired() => {
                let replacement = self.replace_unusable(&first)?;
                self.metadata_attempt(&replacement, request, timeout)
            }
            Err(error) => Err(error),
        }
    }

    pub(super) fn retry_safe<T>(
        self: &Arc<Self>,
        operation: impl Fn(&Arc<Mux>) -> io::Result<T>,
    ) -> io::Result<T> {
        let first = self.mux()?;
        match operation(&first) {
            Ok(value) => Ok(value),
            Err(_error) if first.is_closed() || first.is_retired() => {
                let replacement = self.replace_unusable(&first)?;
                operation(&replacement)
            }
            Err(error) => Err(error),
        }
    }

    /// Dispatch a mutation exactly once. A missing response is ambiguous: the
    /// generation is retired, but the request is never replayed.
    pub(super) fn mutation_call(self: &Arc<Self>, request: Frame) -> io::Result<(Arc<Mux>, Frame)> {
        let mux = self.mux()?;
        match mux.call(request) {
            Ok(frame) => Ok((mux, frame)),
            Err(error) if mux.is_retired() && !mux.is_closed() => Err(error),
            Err(error) => {
                self.invalidate(&mux);
                Err(error)
            }
        }
    }

    pub(super) fn mutation_mux(self: &Arc<Self>) -> io::Result<Arc<Mux>> {
        self.mux()
    }

    pub(super) fn invalidate(&self, mux: &Mux) {
        mux.close();
    }

    fn metadata_attempt(
        &self,
        mux: &Arc<Mux>,
        request: Frame,
        timeout: Duration,
    ) -> io::Result<Frame> {
        let result = mux.call_absolute_timeout(request, timeout);
        if self.reconnect.is_some()
            && matches!(&result, Err(error) if error.kind() == io::ErrorKind::TimedOut)
        {
            mux.retire();
        }
        result
    }

    fn current_mux(&self) -> Arc<Mux> {
        self.state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .current
            .mux
            .clone()
    }

    fn replace_unusable(self: &Arc<Self>, observed: &Arc<Mux>) -> io::Result<Arc<Mux>> {
        let mut state = self
            .state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if !Arc::ptr_eq(&state.current.mux, observed)
            && !state.current.mux.is_closed()
            && !state.current.mux.is_retired()
        {
            return Ok(state.current.mux.clone());
        }
        if !state.current.mux.is_closed() && !state.current.mux.is_retired() {
            return Ok(state.current.mux.clone());
        }
        let reconnect = self.reconnect.as_ref().ok_or_else(|| {
            io::Error::new(
                io::ErrorKind::BrokenPipe,
                "agent transport closed or retired and reconnect is unavailable",
            )
        })?;
        let (mux, _) = establish(reconnect()?, self.heartbeat.deadline)?;
        let id = state.next_id;
        state.next_id = state.next_id.saturating_add(1);
        state.current = TransportGeneration { mux: mux.clone() };
        drop(state);
        if let Err(error) = self.start_heartbeat(mux.clone(), id) {
            mux.close();
            return Err(error);
        }
        Ok(mux)
    }

    #[cfg(test)]
    pub(super) fn replace_observed_for_test(
        self: &Arc<Self>,
        observed: &Arc<Mux>,
    ) -> io::Result<Arc<Mux>> {
        self.replace_unusable(observed)
    }

    fn start_heartbeat(self: &Arc<Self>, mux: Arc<Mux>, id: u64) -> io::Result<()> {
        let weak = Arc::downgrade(self);
        let policy = self.heartbeat;
        std::thread::Builder::new()
            .name(format!("agent-heartbeat-{id}"))
            .spawn(move || heartbeat_loop(weak, mux, policy))?;
        Ok(())
    }
}

fn establish((r, w): AgentStreams, handshake_deadline: Duration) -> io::Result<(Arc<Mux>, String)> {
    let (out_tx, out_rx) = make_out_channel();
    let pending = Arc::new(Mutex::new(HashMap::new()));
    let closed = Arc::new(AtomicBool::new(false));
    let mux = Arc::new(Mux::new_with_stall_timeout(
        out_tx,
        pending.clone(),
        closed.clone(),
        handshake_deadline,
    ));

    let pending_w = pending.clone();
    let closed_w = closed.clone();
    let activity_w = mux.activity();
    std::thread::Builder::new()
        .name("agent-writer".into())
        .spawn(move || {
            let mut w = w;
            loop {
                if closed_w.load(Ordering::Acquire) {
                    break;
                }
                match out_rx.recv_timeout(Duration::from_millis(100)) {
                    Ok((id, frame)) => {
                        if agent_proto::write_frame(&mut w, id, &frame).is_err() {
                            break;
                        }
                        activity_w.touch();
                    }
                    Err(crossbeam_channel::RecvTimeoutError::Timeout) => {}
                    Err(crossbeam_channel::RecvTimeoutError::Disconnected) => break,
                }
            }
            close_transport(&closed_w, &pending_w);
        })?;

    let pending_r = pending.clone();
    let closed_r = closed.clone();
    let activity_r = mux.activity();
    if let Err(error) = std::thread::Builder::new()
        .name("agent-reader".into())
        .spawn(move || {
            let mut r = r;
            loop {
                if !route_frame(&pending_r, &activity_r, agent_proto::read_frame(&mut r)) {
                    break;
                }
            }
            close_transport(&closed_r, &pending_r);
        })
    {
        close_transport(&closed, &pending);
        return Err(error);
    }

    let version = match mux.call_inactivity_timeout(
        Frame::Hello {
            proto: agent_proto::PROTO_VERSION,
        },
        handshake_deadline,
    )? {
        Frame::HelloOk { proto, version } if proto == agent_proto::PROTO_VERSION => version,
        Frame::HelloOk { proto, .. } => {
            mux.close();
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                format!("agent protocol {proto} != {}", agent_proto::PROTO_VERSION),
            ));
        }
        other => {
            mux.close();
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                format!("unexpected handshake reply: {other:?}"),
            ));
        }
    };
    Ok((mux, version))
}

fn heartbeat_loop(
    connection: std::sync::Weak<AgentConnection>,
    mux: Arc<Mux>,
    policy: HeartbeatPolicy,
) {
    loop {
        if connection.upgrade().is_none() {
            mux.close();
            return;
        }
        if mux.is_retired() {
            if mux.is_closed() {
                if let Some(connection) = connection.upgrade() {
                    let _ = connection.replace_unusable(&mux);
                }
                return;
            }
            std::thread::sleep(Duration::from_millis(100));
            continue;
        }
        if mux.is_closed() {
            if let Some(connection) = connection.upgrade() {
                let _ = connection.replace_unusable(&mux);
            }
            return;
        }
        let idle = mux.idle_for();
        if idle < policy.idle {
            std::thread::sleep((policy.idle - idle).min(Duration::from_millis(100)));
            continue;
        }

        // Hello is stateless in the server dispatcher and is deliberately
        // accepted more than once, making it a backwards-compatible ping.
        let response = mux.call_inactivity_timeout(
            Frame::Hello {
                proto: agent_proto::PROTO_VERSION,
            },
            policy.deadline,
        );
        let live = matches!(
            response,
            Ok(Frame::HelloOk { proto, .. }) if proto == agent_proto::PROTO_VERSION
        ) || matches!(response, Ok(Frame::Err(_)));
        if live {
            continue;
        }

        if mux.is_retired() {
            continue;
        }

        // Closing the mux disconnects every pending operation visibly. The
        // replacement opens a fresh `se-agent --serve` channel; no request from
        // this generation is copied to it.
        mux.close();
        if let Some(connection) = connection.upgrade() {
            let _ = connection.replace_unusable(&mux);
        }
        return;
    }
}
