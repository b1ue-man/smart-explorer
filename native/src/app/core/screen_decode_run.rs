//! Decoder/transport run anchor, including a blocked next() call.
use super::screen_decode_thread::DecodeState;
use super::screen_session::{refusal_end, Backend, EndInfo, Outgoing, Shared, StreamLink};
use crate::screen::core::wire_control::ViewerFrame;
use crate::{daemon::StreamRequest, share::PeerOpenTarget};
use std::{
    io,
    sync::{mpsc, Arc},
};
pub(super) fn run(
    backend: Backend,
    peer: PeerOpenTarget,
    request: StreamRequest,
    shared: Arc<Shared>,
    (outbox, inbox): (mpsc::Sender<Outgoing>, mpsc::Receiver<Outgoing>),
    take_control: bool,
) {
    let wake = backend.wake.clone();
    let link = match (backend.open)(peer, request, &shared) {
        Ok(link) => link,
        Err(message) => {
            shared.end(refusal_end(&message));
            wake();
            return;
        }
    };
    let StreamLink {
        memory,
        mut next,
        send,
        close,
    } = link;
    if shared
        .presentation
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .closed
    {
        shared.retirement.watch_memory(&memory);
        shared.retirement.close();
    } else {
        *shared
            .memory
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = memory.clone();
    }
    spawn_sender(inbox, send.clone(), close, shared.clone());
    let mut state = DecodeState::new(shared, backend.decoder.clone(), send, take_control);
    loop {
        match next() {
            Ok(event) => {
                let last = state.on_event(event);
                wake();
                if last {
                    break;
                }
            }
            Err(_) => {
                state.connection_lost();
                wake();
                break;
            }
        }
    }
    drop(state);
    drop(next);
    drop(memory);
    super::screen_render_owner::notify_all();
    // Lets the sender thread go.
    let _ = outbox.send(Outgoing::Close);
}

pub(super) type SendFrame = Arc<dyn Fn(&ViewerFrame) -> io::Result<()> + Send + Sync>;

/// Starts the session's threads; the stream opens on the first of them.
pub(crate) fn spawn(
    backend: Backend,
    peer: PeerOpenTarget,
    request: StreamRequest,
    shared: Arc<Shared>,
    (outbox, inbox): (mpsc::Sender<Outgoing>, mpsc::Receiver<Outgoing>),
    take_control: bool,
) {
    let (failed, wake) = (shared.clone(), backend.wake.clone());
    let started = std::thread::Builder::new()
        .name("screen-viewer-decode".into())
        .spawn(move || {
            super::screen_decode_run::run(
                backend,
                peer,
                request,
                shared,
                (outbox, inbox),
                take_control,
            )
        });
    if let Err(error) = started {
        failed.end(EndInfo {
            text: format!("Die Sitzung konnte nicht starten: {error}"),
            code: None,
        });
        wake();
    }
}

/// Writes the queued frames to the host; ends with the session.
fn spawn_sender(
    inbox: mpsc::Receiver<Outgoing>,
    send: SendFrame,
    close: Arc<dyn Fn() -> io::Result<()> + Send + Sync>,
    shared: Arc<Shared>,
) {
    let _ = std::thread::Builder::new()
        .name("screen-viewer-send".into())
        .spawn(move || {
            while let Ok(item) = inbox.recv() {
                match item {
                    Outgoing::Frame(mut frame) => {
                        shared.decoder_mode(&mut frame);
                        if send(&frame).is_err() {
                            break;
                        }
                    }
                    Outgoing::Close => break,
                }
            }
            let _ = close();
        });
}
