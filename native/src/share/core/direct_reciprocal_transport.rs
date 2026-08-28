use std::io;
use std::sync::{Arc, Mutex};

use iroh::endpoint::{Connection, RecvStream, SendStream};
use tokio::io::AsyncWriteExt;
use tokio::sync::Semaphore;
use zeroize::Zeroizing;

use super::direct_reciprocal_session::{
    DirectRepairInitiator, DirectRepairInitiatorAwaitingStore,
    DirectRepairReceiver, DirectRepairReceiverAwaitingStore, DirectRepairSessionError,
};
use super::direct_reciprocal_store::{DirectRepairStore, DirectRepairStoreError};
use super::direct_reciprocal_wire::{
    decode_direct_repair_frame, encode_direct_repair_frame, DirectRepairMessage,
    DirectRepairPersisted, MAX_DIRECT_REPAIR_FRAME,
};
use super::framing::{send_ctrl, TAG_CTRL};
use super::session::AuthorizedDirectRepair;
use super::wire::Ctrl;

/// Shared serialized access to the exact-CAS relation persistence adapter.
pub(crate) type SharedDirectRepairStore = Arc<Mutex<Box<dyn DirectRepairStore>>>;

pub(crate) fn shared_direct_repair_store(
    store: impl DirectRepairStore + 'static,
) -> SharedDirectRepairStore {
    Arc::new(Mutex::new(Box::new(store)))
}

/// Redacted coordinator result. It intentionally carries no peer identity,
/// relation material, dynamic error text, or transport diagnostics.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum DirectReciprocalTransportResult {
    Complete,
    AlreadyComplete,
    Unsupported,
    Transient,
    PolicyDenied,
    Conflict,
}

pub(super) async fn run_outgoing(
    connection: Connection,
    authorized: AuthorizedDirectRepair,
    store: SharedDirectRepairStore,
) -> DirectReciprocalTransportResult {
    let (state, hello) = match DirectRepairInitiator::begin(
        authorized.local_identity,
        &authorized.local_material,
        authorized.session,
        authorized.expected_remote_material,
    ) {
        Ok(value) => value,
        Err(error) => return classify_session_error(error),
    };
    let (mut send, mut recv) = match connection.open_bi().await {
        Ok(streams) => streams,
        Err(_) => return DirectReciprocalTransportResult::Transient,
    };
    if send_ctrl(&mut send, &Ctrl::DirectReciprocal)
        .await
        .is_err()
    {
        return classify_early_stream_failure(&connection);
    }
    if send_message(&mut send, DirectRepairMessage::Hello(hello))
        .await
        .is_err()
    {
        return classify_early_stream_failure(&connection);
    }
    let offer = match recv_message(&mut recv).await {
        Ok(DirectRepairMessage::Offer(offer)) => offer,
        Ok(_) | Err(RepairReadError::Protocol) => {
            return DirectReciprocalTransportResult::Conflict;
        }
        Err(RepairReadError::Stream | RepairReadError::LegacyControl) => {
            return classify_early_stream_failure(&connection);
        }
    };
    let state = match state.accept_offer(offer) {
        Ok(state) => state,
        Err(error) => return classify_session_error(error),
    };
    let (state, commit) = match persist_initiator(state, store).await {
        Ok(value) => value,
        Err(error) => return classify_session_error(error),
    };
    if send_message(&mut send, DirectRepairMessage::Commit(commit))
        .await
        .is_err()
        || finish_send(&mut send).is_err()
    {
        return DirectReciprocalTransportResult::Transient;
    }
    let complete = match recv_message(&mut recv).await {
        Ok(DirectRepairMessage::Complete(complete)) => complete,
        Ok(_) | Err(RepairReadError::Protocol | RepairReadError::LegacyControl) => {
            return DirectReciprocalTransportResult::Conflict;
        }
        Err(RepairReadError::Stream) => return DirectReciprocalTransportResult::Transient,
    };
    let complete = match state.accept_complete(complete) {
        Ok(complete) => complete,
        Err(error) => return classify_session_error(error),
    };
    if wait_for_ack(&mut send).await.is_err() {
        return DirectReciprocalTransportResult::Transient;
    }
    if complete.receiver_persisted == DirectRepairPersisted::AlreadyComplete
        && complete.initiator_persisted == DirectRepairPersisted::AlreadyComplete
    {
        DirectReciprocalTransportResult::AlreadyComplete
    } else {
        DirectReciprocalTransportResult::Complete
    }
}

pub(super) async fn serve_incoming(
    mut send: SendStream,
    mut recv: RecvStream,
    authorized: AuthorizedDirectRepair,
    store: SharedDirectRepairStore,
) -> io::Result<DirectReciprocalTransportResult> {
    let receiver = DirectRepairReceiver::new(
        authorized.local_identity,
        authorized.local_material,
        authorized.session,
        authorized.expected_remote_material,
    )
    .map_err(session_io_error)?;
    let hello = match recv_message(&mut recv).await {
        Ok(DirectRepairMessage::Hello(hello)) => hello,
        Ok(_) | Err(RepairReadError::Protocol | RepairReadError::LegacyControl) => {
            return Err(protocol_io_error());
        }
        Err(RepairReadError::Stream) => return Err(stream_io_error()),
    };
    let state = receiver.accept_hello(hello).map_err(session_io_error)?;
    let (state, offer) = persist_receiver(state, store).await.map_err(session_io_error)?;
    send_message(&mut send, DirectRepairMessage::Offer(offer))
        .await
        .map_err(|_| stream_io_error())?;
    let commit = match recv_message(&mut recv).await {
        Ok(DirectRepairMessage::Commit(commit)) => commit,
        Ok(_) | Err(RepairReadError::Protocol | RepairReadError::LegacyControl) => {
            return Err(protocol_io_error());
        }
        Err(RepairReadError::Stream) => return Err(stream_io_error()),
    };
    let (complete, message) = state.accept_commit(commit).map_err(session_io_error)?;
    send_message(&mut send, DirectRepairMessage::Complete(message))
        .await
        .map_err(|_| stream_io_error())?;
    finish_send(&mut send)?;
    wait_for_ack(&mut send).await?;
    Ok(if complete.receiver_persisted == DirectRepairPersisted::AlreadyComplete
        && complete.initiator_persisted == DirectRepairPersisted::AlreadyComplete
    {
        DirectReciprocalTransportResult::AlreadyComplete
    } else {
        DirectReciprocalTransportResult::Complete
    })
}

pub(super) async fn serve_incoming_bounded(
    send: SendStream,
    recv: RecvStream,
    authorized: AuthorizedDirectRepair,
    store: SharedDirectRepairStore,
    slots: Arc<Semaphore>,
    transition_slot: Arc<Semaphore>,
    events: crossbeam_channel::Sender<super::types::ShareEvent>,
) -> io::Result<()> {
    let deadline = tokio::time::Instant::now() + super::io_deadline::PEER_OP_TIMEOUT;
    let slot = super::io_deadline::run_until(
        deadline,
        "reciprocal Direct exchange timed out",
        async {
            slots
                .acquire_owned()
                .await
                .map_err(|_| io::Error::other("reciprocal Direct limiter closed"))
        },
    )
    .await?;
    let transition = super::io_deadline::run_until(
        deadline,
        "reciprocal Direct exchange timed out",
        async {
            transition_slot
                .acquire_owned()
                .await
                .map_err(|_| io::Error::other("runtime transition limiter closed"))
        },
    )
    .await?;
    super::io_deadline::run_until(
        deadline,
        "reciprocal Direct exchange timed out",
        serve_incoming(send, recv, authorized, store),
    )
    .await?;
    drop(transition);
    drop(slot);
    events
        .send(super::types::ShareEvent::RuntimeProfilesCommitted)
        .map_err(|_| io::Error::other("Share event receiver closed"))
}

async fn persist_initiator(
    state: DirectRepairInitiatorAwaitingStore,
    store: SharedDirectRepairStore,
) -> Result<
    (
        super::direct_reciprocal_session::DirectRepairInitiatorAwaitingComplete,
        super::direct_reciprocal_wire::DirectRepairCommit,
    ),
    DirectRepairSessionError,
> {
    tokio::task::spawn_blocking(move || {
        let mut store = store.lock().map_err(|_| {
            DirectRepairSessionError::Store(DirectRepairStoreError::Unavailable)
        })?;
        state.persist_with(store.as_mut())
    })
    .await
    .map_err(|_| DirectRepairSessionError::Store(DirectRepairStoreError::Unavailable))?
}

async fn persist_receiver(
    state: DirectRepairReceiverAwaitingStore,
    store: SharedDirectRepairStore,
) -> Result<
    (
        super::direct_reciprocal_session::DirectRepairReceiverAwaitingCommit,
        super::direct_reciprocal_wire::DirectRepairOffer,
    ),
    DirectRepairSessionError,
> {
    tokio::task::spawn_blocking(move || {
        let mut store = store.lock().map_err(|_| {
            DirectRepairSessionError::Store(DirectRepairStoreError::Unavailable)
        })?;
        state.persist_with(store.as_mut())
    })
    .await
    .map_err(|_| DirectRepairSessionError::Store(DirectRepairStoreError::Unavailable))?
}

async fn send_message(
    send: &mut SendStream,
    message: DirectRepairMessage,
) -> Result<(), ()> {
    let frame = encode_direct_repair_frame(&message).map_err(|_| ())?;
    send.write_all(frame.as_bytes()).await.map_err(|_| ())?;
    send.flush().await.map_err(|_| ())
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum RepairReadError {
    Stream,
    LegacyControl,
    Protocol,
}

async fn recv_message(recv: &mut RecvStream) -> Result<DirectRepairMessage, RepairReadError> {
    let mut length = [0_u8; 4];
    recv.read_exact(&mut length)
        .await
        .map_err(|_| RepairReadError::Stream)?;
    let payload_len = u32::from_be_bytes(length) as usize;
    if payload_len == 0 || payload_len > MAX_DIRECT_REPAIR_FRAME {
        return Err(RepairReadError::Protocol);
    }
    let frame_len = payload_len
        .checked_add(4)
        .ok_or(RepairReadError::Protocol)?;
    let mut frame = Zeroizing::new(vec![0_u8; frame_len]);
    frame[..4].copy_from_slice(&length);
    if recv.read_exact(&mut frame[4..]).await.is_err() {
        return Err(RepairReadError::Stream);
    }
    if frame[4] == TAG_CTRL {
        return Err(RepairReadError::LegacyControl);
    }
    decode_direct_repair_frame(std::mem::take(&mut *frame))
        .map_err(|_| RepairReadError::Protocol)
}

fn finish_send(send: &mut SendStream) -> io::Result<()> {
    send.finish().map_err(|_| stream_io_error())
}

async fn wait_for_ack(send: &mut SendStream) -> io::Result<()> {
    send.stopped().await.map_err(|_| stream_io_error())?;
    Ok(())
}

fn classify_early_stream_failure(connection: &Connection) -> DirectReciprocalTransportResult {
    if connection.close_reason().is_none() {
        DirectReciprocalTransportResult::Unsupported
    } else {
        DirectReciprocalTransportResult::Transient
    }
}

fn classify_session_error(error: DirectRepairSessionError) -> DirectReciprocalTransportResult {
    match error {
        DirectRepairSessionError::PolicyDenied
        | DirectRepairSessionError::Store(DirectRepairStoreError::PolicyDenied) => {
            DirectReciprocalTransportResult::PolicyDenied
        }
        DirectRepairSessionError::CapabilityNotRequested => {
            DirectReciprocalTransportResult::Unsupported
        }
        DirectRepairSessionError::Store(
            DirectRepairStoreError::Retryable
            | DirectRepairStoreError::StaleLocalIdentity
            | DirectRepairStoreError::Unavailable,
        )
        | DirectRepairSessionError::Wire(
            super::direct_reciprocal_wire::DirectRepairWireError::EntropyUnavailable,
        ) => DirectReciprocalTransportResult::Transient,
        _ => DirectReciprocalTransportResult::Conflict,
    }
}

fn session_io_error(error: DirectRepairSessionError) -> io::Error {
    let kind = match classify_session_error(error) {
        DirectReciprocalTransportResult::PolicyDenied => io::ErrorKind::PermissionDenied,
        DirectReciprocalTransportResult::Transient => io::ErrorKind::WouldBlock,
        _ => io::ErrorKind::InvalidData,
    };
    io::Error::new(kind, "reciprocal Direct stream rejected")
}

fn protocol_io_error() -> io::Error {
    io::Error::new(
        io::ErrorKind::InvalidData,
        "invalid reciprocal Direct stream message",
    )
}

fn stream_io_error() -> io::Error {
    io::Error::new(
        io::ErrorKind::ConnectionReset,
        "reciprocal Direct stream ended",
    )
}
