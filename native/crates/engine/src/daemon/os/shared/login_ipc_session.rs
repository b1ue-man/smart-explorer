//! One owning local socket, one existing Share endpoint and one signed sequence.
use super::registry::{self, Entry, Owner};
use crate::daemon::ipc_protocol::{
    login::{
        self as wire, LoginEnd, LoginFrame, LoginIpcReply, LoginRelayCommand, LoginRelayReply,
        LoginSessionView,
    },
    write_response, IpcResponse,
};
use crate::{
    login::{GatewayTarget, LoginCancellation, RemoteLoginClient},
    share::login_transport::LoginClientContext,
};
use std::{
    io::{self, Write},
    net::{Shutdown, TcpStream},
    sync::Arc,
    time::{Duration, Instant},
};
use tokio::sync::{mpsc, oneshot};
const IDLE: Duration = Duration::from_secs(5);
enum Out {
    Hello(LoginSessionView),
    Reply {
        frame: LoginFrame,
        complete: oneshot::Sender<io::Result<()>>,
    },
    End(LoginEnd),
}
pub(super) fn serve(
    mut socket: TcpStream,
    context: LoginClientContext,
    target: GatewayTarget,
    user: String,
) -> io::Result<()> {
    let runtime = context.runtime.clone();
    let cancel = Arc::new(LoginCancellation::default());
    let session = crate::login::os::shared::nonce()?;
    let owner = registry::register(
        LoginSessionView {
            session: session.clone(),
            target: target.clone(),
            target_user: user.clone(),
            state: "connecting".into(),
            challenge: None,
            access: None,
            route: None,
        },
        cancel.clone(),
        socket.try_clone()?,
    )?;
    write_response(
        &mut socket,
        &IpcResponse::Login(LoginIpcReply::StreamOpened { session }),
    )?;
    socket.set_read_timeout(None)?;
    socket.set_write_timeout(Some(IDLE))?;
    let (commands, input) = mpsc::channel(1);
    let (output, mut frames) = mpsc::channel(1);
    let mut reader_socket = socket.try_clone()?;
    let read_cancel = cancel.clone();
    let reader = std::thread::Builder::new()
        .name("login-viewer-input".into())
        .spawn(move || {
            read_commands(&mut reader_socket, commands, read_cancel);
        })?;
    let task_cancel = cancel.clone();
    let (finished, finish) = std::sync::mpsc::sync_channel(1);
    let task_owner = owner.clone();
    runtime.spawn(async move {
        let result = run(&task_owner, context, target, user, input, &output).await;
        if let Err(error) = result {
            let _ = output.try_send(Out::End(LoginEnd {
                code: "login_ended".into(),
                detail: Some(error.to_string()),
            }));
        }
        drop(output);
        drop(task_owner);
        let _ = finished.send(());
    });
    let result = (|| {
        while let Some(frame) = frames.blocking_recv() {
            match frame {
                Out::Hello(view) => wire::write_json(&mut socket, wire::HELLO, &view)?,
                Out::Reply { frame, complete } => {
                    write_owned(&mut socket, frame, complete)?;
                }
                Out::End(end) => wire::write_json(&mut socket, wire::END, &end)?,
            }
        }
        Ok(())
    })();
    drop(frames);
    task_cancel.cancel();
    let _ = socket.shutdown(Shutdown::Both);
    let _ = reader.join();
    // Cancellation drops the in-flight signed future, never regenerates it.
    // Keep this owning handler until the bounded async bridge has released its client.
    finish
        .recv_timeout(Duration::from_secs(35))
        .map_err(io::Error::other)?;
    result
}
pub(super) fn write_owned(
    writer: &mut impl Write,
    frame: LoginFrame,
    complete: oneshot::Sender<io::Result<()>>,
) -> io::Result<()> {
    let written = wire::write_reply(writer, &frame);
    // A next producer sees completion only after the actual image borrow and
    // its final writer owner have ended, including a failed partial write.
    drop(frame);
    let result = written
        .as_ref()
        .map(|_| ())
        .map_err(|e| io::Error::new(e.kind(), e.to_string()));
    let _ = complete.send(result);
    written
}
fn read_commands(
    socket: &mut TcpStream,
    commands: mpsc::Sender<LoginRelayCommand>,
    cancel: Arc<LoginCancellation>,
) {
    // One bounded queued command avoids a race between the client reading the
    // final response byte and the writer recording completion. The async owner
    // never starts it before that completion. CLOSE still bypasses this queue;
    // a full queue cancels/shuts down rather than blocking input release.
    while let Ok((tag, bytes)) = wire::read_frame(socket, wire::MAX_CONTROL) {
        if tag == wire::CLOSE && bytes.is_empty() {
            break;
        }
        if tag != wire::COMMAND {
            break;
        }
        let command: LoginRelayCommand = match serde_json::from_slice(&bytes) {
            Ok(command) => command,
            Err(_) => break,
        };
        if command.command.validate().is_err() || commands.try_send(command).is_err() {
            break;
        }
    }
    cancel.cancel();
    let _ = socket.shutdown(Shutdown::Both);
}
async fn deliver(output: &mpsc::Sender<Out>, frame: Out) -> io::Result<()> {
    tokio::time::timeout(IDLE, output.send(frame))
        .await
        .map_err(|_| {
            io::Error::new(
                io::ErrorKind::TimedOut,
                "login local viewer stopped receiving",
            )
        })?
        .map_err(|_| io::Error::new(io::ErrorKind::BrokenPipe, "login local viewer closed"))
}
async fn deliver_reply(
    entry: &Entry,
    output: &mpsc::Sender<Out>,
    frame: LoginFrame,
) -> io::Result<()> {
    let (complete, written) = oneshot::channel();
    deliver(output, Out::Reply { frame, complete }).await?;
    tokio::select! {
        biased;
        () = entry.cancel.cancelled() => Err(io::Error::new(io::ErrorKind::Interrupted,"login local write cancelled")),
        result = written => result.map_err(|_|io::Error::new(io::ErrorKind::BrokenPipe,"login writer ended without completion"))?,
    }
}
fn update(entry: &Entry, client: &RemoteLoginClient, state: &str) -> io::Result<LoginSessionView> {
    let mut view = entry
        .view
        .lock()
        .map_err(|_| io::Error::other("login session state is unavailable"))?;
    view.state = state.into();
    view.challenge = client.challenge().ok().cloned();
    view.access = Some(client.access().clone());
    view.route = client.route_status();
    Ok(view.clone())
}
async fn run(
    owner: &Owner,
    context: LoginClientContext,
    target: GatewayTarget,
    user: String,
    mut input: mpsc::Receiver<LoginRelayCommand>,
    output: &mpsc::Sender<Out>,
) -> io::Result<()> {
    let entry = &owner.entry;
    let lifetime: Arc<dyn crate::screen::core::capture_grant::CaptureCreditLifetime> =
        Arc::new(owner.clone());
    let mut client = RemoteLoginClient::connect_async_owned(
        context,
        target,
        user,
        entry.cancel.clone(),
        Some(lifetime),
    )
    .await?;
    deliver(output, Out::Hello(update(entry, &client, "challenge")?)).await?;
    let mut sequence = 1u64;
    let mut lease = Instant::now() + IDLE;
    loop {
        let next = tokio::select! {
            biased;
            () = entry.cancel.cancelled() => return Ok(()),
            () = client.wait_closed() => return Err(io::Error::new(io::ErrorKind::UnexpectedEof, "login gateway closed")),
            _ = tokio::time::sleep_until(lease.into()) => return Err(io::Error::new(io::ErrorKind::TimedOut, "login signed-operation idle lease ended")),
            next = input.recv() => next,
        };
        let Some(command) = next else {
            return Ok(());
        };
        if command.sequence != sequence {
            return Err(wire::invalid("login local command sequence mismatch"));
        }
        sequence = sequence
            .checked_add(1)
            .ok_or_else(|| wire::invalid("login local sequence exhausted"))?;
        let packet = if let Some(operation) = command.command.operation() {
            let request = client.signed_request(operation)?;
            let (packet, image) = client.signed_operation_async(request).await?;
            // Only the completion of an explicitly requested signed operation
            // begins another local wait. Status/challenge cannot keep it alive.
            lease = Instant::now() + IDLE;
            let terminal = packet.reply.challenge.is_none();
            let view = update(entry, &client, &packet.reply.state)?;
            deliver_reply(
                entry,
                output,
                LoginFrame {
                    reply: LoginRelayReply {
                        sequence: command.sequence,
                        view,
                        packet: Some(packet),
                    },
                    image,
                },
            )
            .await?;
            if terminal {
                return Ok(());
            }
            continue;
        } else {
            None
        };
        let view = update(entry, &client, "challenge")?;
        deliver_reply(
            entry,
            output,
            LoginFrame {
                reply: LoginRelayReply {
                    sequence: command.sequence,
                    view,
                    packet,
                },
                image: None,
            },
        )
        .await?;
    }
}
