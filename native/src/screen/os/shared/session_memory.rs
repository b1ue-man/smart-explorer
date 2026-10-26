//! One process-wide accounting pool for viewer and host session promises.
use crate::screen::core::{
    frame_budget::MAX_SESSION_WORKING_BYTES,
    frame_memory::{self, CapacityCeiling, MemoryFacts, MemoryLease},
};
pub(crate) fn facts() -> Option<MemoryFacts> {
    super::platform_session_memory::facts()
}
pub(crate) fn capacity_ceiling(facts: Option<MemoryFacts>) -> CapacityCeiling {
    super::platform_session_memory::ceiling(facts)
}
pub(crate) fn observe_platform_ceiling(ceiling: CapacityCeiling) {
    super::platform_session_memory::observe_ceiling(ceiling);
}
pub(crate) fn reserve(requested: u64) -> std::io::Result<MemoryLease> {
    let facts = facts();
    frame_memory::reserve_with_ceiling(
        facts,
        capacity_ceiling(facts),
        requested.min(MAX_SESSION_WORKING_BYTES),
    )
}
#[cfg(test)]
#[path = "batches_memory_task_tests.rs"]
mod tests;

/// Platform admission runs before the serialized ClientHello; the core sees
/// only the narrowed offer. All production IPC viewers use this entrypoint.
pub(crate) async fn handshake(
    connection: iroh::endpoint::Connection,
    build: impl FnOnce(
        &crate::screen::core::wire_hello::ScreenServerHello,
    ) -> std::io::Result<crate::screen::core::wire_hello::ScreenClientHello>,
    on_pending: impl FnMut(crate::share::access_grant::Capability),
) -> Result<
    crate::screen::core::viewer_client::ViewerSession,
    crate::screen::core::viewer_client::ViewerError,
> {
    let admitted = |server: &crate::screen::core::wire_hello::ScreenServerHello| {
        admit_hello(
            build(server)?,
            server,
            super::platform_session_memory::preview_admission(),
            reserve,
        )
    };
    crate::screen::core::viewer_client::handshake_admitted(connection, admitted, on_pending).await
}

fn admit_hello(
    mut hello: crate::screen::core::wire_hello::ScreenClientHello,
    server: &crate::screen::core::wire_hello::ScreenServerHello,
    preview: crate::screen::core::frame_budget::PreviewAdmission,
    reserve: impl FnOnce(u64) -> std::io::Result<MemoryLease>,
) -> std::io::Result<(
    crate::screen::core::wire_hello::ScreenClientHello,
    frame_memory::MemoryHold,
)> {
    use crate::screen::core::frame_budget::{self, PreviewAdmission};
    if hello.mode == crate::screen::core::wire_hello::SessionMode::Probe {
        hello.frame_capabilities = None;
        return Ok((hello, None));
    }
    let caps = hello
        .frame_capabilities
        .map(|caps| caps.for_server(server.frame_memory_class))
        .transpose()?;
    let offered = caps.map_or(frame_budget::MAX_WORKING_BYTES, |caps| {
        caps.frames.working_bytes
    });
    let geometric = preview == PreviewAdmission::Geometric;
    let requested = if geometric {
        frame_budget::requested_bytes(&hello, offered)?
    } else {
        offered
    };
    let lease = reserve(requested)?;
    if let Some(caps) = caps {
        let caps = if geometric {
            frame_budget::preview_capabilities(caps, &hello)?
        } else {
            caps
        };
        hello.frame_capabilities = Some(caps.with_working_budget(lease.bytes())?);
    } else {
        frame_budget::legacy_limits(lease.bytes(), None)?;
    }
    Ok((hello, Some(std::sync::Arc::new(lease))))
}

#[cfg(test)]
#[path = "batches_session_budget_compatibility_task_tests.rs"]
mod compatibility_tests;
