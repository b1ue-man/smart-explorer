//! The channel between the shell and one surface (C5.5, Ä14): frames of
//! `core::protocol` over the surface's stdin/stdout. Both ends read in a
//! thread of their own and write through a bounded queue drained by a thread
//! of their own, so neither the shell nor a surface's UI thread ever blocks
//! on the other side. On the surface the end of the stream ends the process
//! with exit code 76 from the reader thread, independent of the UI thread; a
//! `stop` ends it with 75 (update) or 0. Under GNOME the streams are the
//! surface's connection to `desktop.sock` instead (`RemoteEnd`, DSK-M8).
use std::io::{self, Read, Write};
use std::process::Child;
use std::sync::mpsc::{self, Receiver, Sender, SyncSender};

use serde::de::DeserializeOwned;
use serde::Serialize;

use crate::daemon::EXIT_UPDATE_STOP;
use crate::desktop::core::protocol::{
    read_frame, write_frame, ShellMessage, StopReason, SurfaceMessage, EXIT_CHANNEL_END,
};

/// Messages waiting for the writer thread; a full queue drops the newest.
const QUEUE: usize = 16;

/// The writing end: queues messages for a writer thread.
pub(crate) struct FrameSender<T> {
    queue: SyncSender<T>,
}

impl<T: Serialize + Send + 'static> FrameSender<T> {
    pub(crate) fn spawn<W: Write + Send + 'static>(
        writer: W,
        name: &str,
    ) -> io::Result<FrameSender<T>> {
        let (queue, pending) = mpsc::sync_channel::<T>(QUEUE);
        std::thread::Builder::new()
            .name(name.into())
            .spawn(move || {
                let mut writer = writer;
                for message in pending {
                    if write_frame(&mut writer, &message).is_err() {
                        return;
                    }
                }
            })?;
        Ok(FrameSender { queue })
    }

    /// Queues `message`; `false` when the queue is full or the writer ended.
    pub(crate) fn send(&self, message: T) -> bool {
        self.queue.try_send(message).is_ok()
    }
}

/// What a reader thread reports.
#[derive(Debug)]
pub(crate) enum LinkEvent<M> {
    Message(M),
    /// The stream ended (`None`) or broke (the error); always the last event.
    Ended(Option<String>),
}

/// Reads frames in a thread and hands every event to `deliver`.
pub(crate) fn spawn_reader<R, M, F>(reader: R, name: &str, deliver: F) -> io::Result<()>
where
    R: Read + Send + 'static,
    M: DeserializeOwned + Send + 'static,
    F: FnMut(LinkEvent<M>) + Send + 'static,
{
    std::thread::Builder::new()
        .name(name.into())
        .spawn(move || {
            let mut reader = reader;
            let mut deliver = deliver;
            loop {
                match read_frame::<_, M>(&mut reader) {
                    Ok(Some(message)) => deliver(LinkEvent::Message(message)),
                    Ok(None) => return deliver(LinkEvent::Ended(None)),
                    Err(error) => return deliver(LinkEvent::Ended(Some(error.to_string()))),
                }
            }
        })
        .map(drop)
}

/// The shell's end for one surface child; events arrive tagged with `id`.
pub(crate) fn shell_side(
    child: &mut Child,
    id: u64,
    events: Sender<(u64, LinkEvent<SurfaceMessage>)>,
) -> io::Result<FrameSender<ShellMessage>> {
    let stdin = child
        .stdin
        .take()
        .ok_or_else(|| io::Error::other("Fläche ohne Eingabekanal"))?;
    let stdout = child
        .stdout
        .take()
        .ok_or_else(|| io::Error::other("Fläche ohne Ausgabekanal"))?;
    let sender = FrameSender::spawn(stdin, "se-desktop-to-surface")?;
    spawn_reader(stdout, "se-desktop-from-surface", move |event| {
        let _ = events.send((id, event));
    })?;
    Ok(sender)
}

/// Exit code of a surface for a `stop` of the shell.
pub(crate) fn stop_exit_code(reason: StopReason) -> i32 {
    match reason {
        StopReason::Update => EXIT_UPDATE_STOP,
        StopReason::Disable | StopReason::Restart => 0,
    }
}

/// The far end of a surface that connected over a socket instead of being
/// started by the shell (the GNOME extension starts its surfaces): where it
/// reads and writes frames, and the process behind it.
pub(crate) struct RemoteEnd {
    pub(crate) reader: Box<dyn Read + Send>,
    pub(crate) writer: Box<dyn Write + Send>,
    pub(crate) pid: u32,
}

/// The shell's end for a surface that connected: events arrive tagged `id`.
pub(crate) fn shell_side_remote(
    end: RemoteEnd,
    id: u64,
    events: Sender<(u64, LinkEvent<SurfaceMessage>)>,
) -> io::Result<FrameSender<ShellMessage>> {
    let sender = FrameSender::spawn(end.writer, "se-desktop-to-remote")?;
    spawn_reader(end.reader, "se-desktop-from-remote", move |event| {
        let _ = events.send((id, event));
    })?;
    Ok(sender)
}

/// The surface's end on any pair of streams. `wake` runs after each
/// delivered message (the UI asks for a frame); `stop` and the end of the
/// stream end the process here.
pub(crate) fn surface_streams<R, W, F>(
    reader: R,
    writer: W,
    wake: F,
) -> io::Result<(FrameSender<SurfaceMessage>, Receiver<ShellMessage>)>
where
    R: Read + Send + 'static,
    W: Write + Send + 'static,
    F: Fn() + Send + 'static,
{
    let sender = FrameSender::spawn(writer, "se-desktop-out")?;
    let (forward, inbox) = mpsc::channel();
    spawn_reader(reader, "se-desktop-in", move |event| match event {
        LinkEvent::Message(ShellMessage::Stop { reason }) => {
            std::process::exit(stop_exit_code(reason))
        }
        LinkEvent::Message(message) => {
            let _ = forward.send(message);
            wake();
        }
        LinkEvent::Ended(_) => std::process::exit(EXIT_CHANNEL_END),
    })?;
    Ok((sender, inbox))
}

/// The surface's end on its own stdin/stdout.
pub(crate) fn surface_stdio<W>(
    wake: W,
) -> io::Result<(FrameSender<SurfaceMessage>, Receiver<ShellMessage>)>
where
    W: Fn() + Send + 'static,
{
    surface_streams(io::stdin(), io::stdout(), wake)
}
