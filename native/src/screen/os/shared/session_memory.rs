//! One process-wide accounting pool for viewer and host session promises.
use crate::screen::core::{frame_budget::MAX_SESSION_WORKING_BYTES,frame_memory::{self,CapacityCeiling,MemoryFacts,MemoryLease}};
pub(crate) fn facts()->Option<MemoryFacts> {super::platform_session_memory::facts()}
pub(crate) fn capacity_ceiling(facts:Option<MemoryFacts>)->CapacityCeiling {super::platform_session_memory::ceiling(facts)}
pub(crate) fn observe_platform_ceiling(ceiling:CapacityCeiling) {super::platform_session_memory::observe_ceiling(ceiling);}
pub(crate) fn reserve(requested:u64)->std::io::Result<MemoryLease> {
    let facts=facts();
    frame_memory::reserve_with_ceiling(facts,capacity_ceiling(facts),requested.min(MAX_SESSION_WORKING_BYTES))
}
#[cfg(test)]
#[path="batches_memory_task_tests.rs"]
mod tests;

/// Platform admission runs before the serialized ClientHello; the core sees
/// only the narrowed offer. All production IPC viewers use this entrypoint.
pub(crate) async fn handshake(
    connection:iroh::endpoint::Connection,
    build:impl FnOnce(&crate::screen::core::wire_hello::ScreenServerHello)->std::io::Result<crate::screen::core::wire_hello::ScreenClientHello>,
    on_pending:impl FnMut(crate::share::access_grant::Capability),
)->Result<crate::screen::core::viewer_client::ViewerSession,crate::screen::core::viewer_client::ViewerError> {
    let admitted=|server:&crate::screen::core::wire_hello::ScreenServerHello| {
        let mut hello=build(server)?;let memory;
        if let Some(caps)=hello.frame_capabilities {
            let caps=caps.for_server(server.frame_memory_class)?;
            let lease=reserve(caps.frames.working_bytes)?;
            hello.frame_capabilities=Some(caps.with_working_budget(lease.bytes())?);
            memory=Some(std::sync::Arc::new(lease));
        } else {
            let bytes=crate::screen::core::frame_budget::MAX_WORKING_BYTES;
            let lease=reserve(bytes)?;
            if lease.bytes()!=bytes {return Err(std::io::Error::other("insufficient memory for legacy screen session"));}
            memory=Some(std::sync::Arc::new(lease));
        }
        Ok((hello,memory))
    };
    crate::screen::core::viewer_client::handshake_admitted(connection,admitted,on_pending).await
}
