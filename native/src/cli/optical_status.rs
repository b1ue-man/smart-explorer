//! Process-local source-reader information shared by terminal and MCP output.
// Source states are read through source_media; no second registry is created.
// This module deliberately has no OS probing entrypoint.

pub(in crate::cli) fn text(states: &[crate::source_media::Status]) -> Option<String> {
    if states.is_empty() { return None; }
    let states: Vec<String> = states.iter().map(|state| {
        let mode = if !state.active { "waiting" } else { match state.mode {
            crate::source_media::ReadMode::Waiting => "waiting",
            crate::source_media::ReadMode::Exclusive => "exclusive",
            crate::source_media::ReadMode::Sequential => "sequential",
            crate::source_media::ReadMode::MetadataFallback => "metadata-preserving fallback",
        }};
        let block = state.block_bytes.map(|bytes| format!(", block {bytes} bytes")).unwrap_or_default();
        format!("{}: {mode}{block}", state.source.device)
    }).collect();
    Some(format!("optical readers in this process: {}", states.join("; ")))
}
