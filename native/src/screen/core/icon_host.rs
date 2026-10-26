//! P3 metadata authorization shared by active viewers and capture-free list channels.
use std::io;
use std::time::Duration;
use iroh::endpoint::{Connection, RecvStream, SendStream};
use super::app_icon::{IconRequest, IconResponse};
use super::authorize::log_event;
use super::host_server::HostContext;
use super::registry::SessionHandle;
use super::window_list::WindowList;
use super::wire_control::{recv_viewer_frame, HostFrame, Pong, ViewerFrame};
use super::wire_hello::SessionMode;
use crate::share::access_grant::Capability;
use crate::share::relation_proof::ProvenPrincipal;

fn denied() -> io::Error {io::Error::new(io::ErrorKind::PermissionDenied,"screen-list permission is no longer valid")}

/// Admission covers the fresh list and the OS lookup; permission cannot change between them.
pub(crate) fn list(context: &HostContext, principal: &ProvenPrincipal) -> io::Result<WindowList> {
    let mut result = None;
    let allowed = context.authority.admit(principal,&[Capability::ScreenView],crate::share::core_now_secs(),&mut || {
        result=Some(context.backend.windows().map(WindowList::bounded));
    });
    if !allowed {return Err(denied());}
    result.unwrap_or_else(||Err(denied()))
}

pub(crate) fn icon(context: &HostContext, principal: &ProvenPrincipal, request: IconRequest) -> io::Result<IconResponse> {
    request.validate()?;
    let mut content = None;
    let allowed = context.authority.admit(principal,&[Capability::ScreenView],crate::share::core_now_secs(),&mut || {
        // A source/list/decoder failure omits this icon and never tears down capture.
        content=(|| {
            let list=context.backend.windows().ok()?.bounded();
            let entry=request.authorize(&list).ok()?;
            let reply=context.backend.icon(entry).ok().flatten()?;
            reply.bytes_for(&request.icon).ok()?;
            Some(reply)
        })();
    });
    if !allowed {return Err(denied());}
    Ok(IconResponse {request,content})
}

/// Only list/icon/ping frames are accepted. No source, encoder or input path exists here.
pub(crate) async fn serve_metadata(
    context: &HostContext, principal: &ProvenPrincipal, connection: &Connection,
    mut send: SendStream, mut recv: RecvStream, handle: SessionHandle,
    families_supported:bool,
) -> io::Result<()> {
    let mut stop=handle.stop_signal();
    handle.update(|view|view.state=super::session_state::SessionState::Streaming);
    let result=loop {
        let frame=tokio::select! {
            changed=stop.changed()=>{let _=changed;break Ok(());},
            frame=tokio::time::timeout(Duration::from_secs(15),recv_viewer_frame(&mut recv))=>match frame {
                Ok(Ok(frame))=>frame,
                Ok(Err(error))=>break Err(error),
                Err(_)=>break Ok(()),
            },
        };
        if stop.borrow().is_some() {break Ok(());}
        let response=match frame {
            ViewerFrame::ListFamilies if families_supported=>super::family_listing::list(context,principal).map(HostFrame::Families),
            ViewerFrame::ListWindows=>list(context,principal).map(HostFrame::Windows),
            ViewerFrame::IconRequest(request)=>icon(context,principal,request).map(HostFrame::IconReply),
            ViewerFrame::Ping(ping)=>{
                let now=super::gop_stream::now_us();
                Ok(HostFrame::Pong(Pong::answer(&ping,now,now)))
            }
            _=>Err(io::Error::new(io::ErrorKind::InvalidInput,"capture and input are unavailable on a metadata channel")),
        };
        let response=match response {Ok(response)=>response,Err(error)=>break Err(error)};
        if stop.borrow().is_some() {break Ok(());}
        let sent=tokio::select! {
            biased;
            _=stop.changed()=>break Ok(()),
            sent=super::control_sender::send(connection.remote_id().to_string(),&mut send,&response)=>sent,
        };
        if let Err(error)=sent {break Err(error);}
    };
    let _=send.finish();
    log_event(context.audit,principal,SessionMode::View,"session_ended",None,Some("metadata channel closed"));
    result
}
