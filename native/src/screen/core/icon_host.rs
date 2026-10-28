//! P3 metadata authorization shared by active viewers and capture-free list channels.
use super::app_icon::{IconRequest, IconResponse};
use super::authorize::log_event;
use super::host_metadata::Request;
use super::host_server::HostContext;
use super::registry::SessionHandle;
use super::session_state::EndReason;
use super::window_list::WindowList;
use super::wire_control::{recv_metadata_frame, HostFrame, MetadataFrame, Pong, ViewerFrame};
use super::wire_hello::SessionMode;
use crate::share::access_grant::Capability;
use crate::share::relation_proof::ProvenPrincipal;
use iroh::endpoint::{Connection, RecvStream, SendStream};
use std::io;
use std::time::Duration;
use tokio::sync::watch;

/// Runs in a metadata job (`host_metadata`): the OS work never holds the
/// shared authorization lock; the rights are checked before and after it.
pub(crate) fn list(context: &HostContext, principal: &ProvenPrincipal) -> io::Result<WindowList> {
    super::host_metadata_jobs::check(context, principal, &[Capability::ScreenView])?;
    let result = context.backend.windows().map(WindowList::bounded);
    super::host_metadata_jobs::check(context, principal, &[Capability::ScreenView])?;
    result
}

pub(crate) fn icon(
    context: &HostContext,
    principal: &ProvenPrincipal,
    request: IconRequest,
) -> io::Result<IconResponse> {
    request.validate()?;
    super::host_metadata_jobs::check(context, principal, &[Capability::ScreenView])?;
    // A source/list/decoder failure omits this icon and never tears down capture.
    let content = (|| {
        let list = context.backend.windows().ok()?.bounded();
        let entry = request.authorize(&list).ok()?;
        super::host_metadata_jobs::check(context, principal, &[Capability::ScreenView]).ok()?;
        let reply = context.backend.icon(entry).ok().flatten()?;
        reply.bytes_for(&request.icon).ok()?;
        Some(reply)
    })();
    super::host_metadata_jobs::check(context, principal, &[Capability::ScreenView])?;
    Ok(IconResponse { request, content })
}

/// One answer from a metadata job; a stop or a leaving viewer cancels it.
async fn answer(
    context: &HostContext,
    principal: &ProvenPrincipal,
    connection: &Connection,
    stop: &watch::Receiver<Option<EndReason>>,
    request: Request,
) -> io::Result<HostFrame> {
    let fallback = request.fallback();
    let mut job = request.start(context, principal);
    let result = super::host_metadata::until(connection, Some(stop.clone()), job.result()).await;
    fallback.settle(result)
}

/// Only list/icon/ping frames are accepted. No source, encoder or input path exists here.
/// `SUBSCRIBE_FEED` turns the channel into the host's device feed (design 5.2).
pub(crate) async fn serve_metadata(
    context: &HostContext,
    principal: &ProvenPrincipal,
    connection: &Connection,
    mut send: SendStream,
    mut recv: RecvStream,
    handle: SessionHandle,
    families_supported: bool,
) -> io::Result<()> {
    let mut stop = handle.stop_signal();
    handle.update(|view| view.state = super::session_state::SessionState::Streaming);
    let result = loop {
        let frame = tokio::select! {
            changed=stop.changed()=>{let _=changed;break Ok(());},
            frame=tokio::time::timeout(Duration::from_secs(15),recv_metadata_frame(&mut recv))=>match frame {
                Ok(Ok(frame))=>frame,
                Ok(Err(error))=>break Err(error),
                Err(_)=>break Ok(()),
            },
        };
        if stop.borrow().is_some() {
            break Ok(());
        }
        let frame = match frame {
            MetadataFrame::Viewer(frame) => frame,
            MetadataFrame::SubscribeFeed => {
                let stop = stop.clone();
                let feed = super::feed_host::run(
                    context,
                    principal,
                    connection,
                    &mut send,
                    recv,
                    stop,
                    families_supported,
                );
                break feed.await;
            }
        };
        let response = match frame {
            ViewerFrame::ListFamilies if families_supported => {
                answer(context, principal, connection, &stop, Request::Families).await
            }
            ViewerFrame::ListWindows => {
                answer(context, principal, connection, &stop, Request::Windows).await
            }
            ViewerFrame::IconRequest(request) => match request.validate() {
                Ok(()) => {
                    let request = Request::Icon(request);
                    answer(context, principal, connection, &stop, request).await
                }
                Err(error) => Err(error),
            },
            ViewerFrame::Ping(ping) => {
                let now = super::gop_stream::now_us();
                Ok(HostFrame::Pong(Pong::answer(&ping, now, now)))
            }
            _ => Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "capture and input are unavailable on a metadata channel",
            )),
        };
        let sent = match response {
            Ok(response) => {
                super::host_metadata::send(
                    context,
                    principal,
                    SessionMode::View,
                    connection,
                    &mut send,
                    &response,
                    stop.clone(),
                )
                .await
            }
            Err(error) => Err(error),
        };
        if let Err(error) = sent {
            // A stop of the session ends the channel as it always did.
            break if stop.borrow().is_some() {
                Ok(())
            } else {
                Err(error)
            };
        }
    };
    let _ = send.finish();
    log_event(
        context.audit,
        principal,
        SessionMode::View,
        "session_ended",
        None,
        Some("metadata channel closed"),
    );
    result
}
