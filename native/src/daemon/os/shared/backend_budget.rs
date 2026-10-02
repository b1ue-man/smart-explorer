use std::io;

const MAX_BACKEND_WALK_NODES: u64 = 1_000_000;
const MAX_BACKEND_WALK_TEXT_BYTES: u64 = 128 * 1024 * 1024;
const MAX_BACKEND_WALK_DEPTH: usize = 512;

/// Bounds of one backend walk the worker serves. A walk that streams its
/// entries (search, hash walk) keeps nothing here: only its depth is bounded,
/// so a cyclic listing still ends, and every client – which bounds its own
/// collection – reaches its own, graceful limit first instead of a failure
/// from this side. A walk that assembles its tree here also bounds the nodes
/// and path text that tree holds.
pub(super) struct WalkBudget {
    nodes: u64,
    text_bytes: u64,
    retains: bool,
}

impl WalkBudget {
    /// For walks that send every entry on at once.
    pub(super) fn streaming() -> Self {
        Self {
            nodes: 0,
            text_bytes: 0,
            retains: false,
        }
    }

    /// For walks that keep a tree until they are done.
    pub(super) fn retaining() -> Self {
        Self {
            retains: true,
            ..Self::streaming()
        }
    }

    pub(super) fn record(&mut self, path: &str, depth: usize) -> io::Result<()> {
        if depth > MAX_BACKEND_WALK_DEPTH {
            return Err(invalid(format!(
                "backend walk exceeds {MAX_BACKEND_WALK_DEPTH} levels"
            )));
        }
        if !self.retains {
            return Ok(());
        }
        self.nodes = self.nodes.saturating_add(1);
        self.text_bytes = self.text_bytes.saturating_add(path.len() as u64);
        if self.nodes > MAX_BACKEND_WALK_NODES || self.text_bytes > MAX_BACKEND_WALK_TEXT_BYTES {
            return Err(invalid(
                "backend walk exceeds its bounded collection budget",
            ));
        }
        Ok(())
    }
}

fn invalid(message: impl Into<String>) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, message.into())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn backend_walk_budget_rejects_excessive_depth() {
        for mut budget in [WalkBudget::streaming(), WalkBudget::retaining()] {
            assert_eq!(
                budget
                    .record("deep", MAX_BACKEND_WALK_DEPTH + 1)
                    .unwrap_err()
                    .kind(),
                io::ErrorKind::InvalidData
            );
        }
    }

    #[test]
    fn review_task_streaming_walks_leave_the_entry_limit_to_the_client() {
        let mut streaming = WalkBudget::streaming();
        let mut retaining = WalkBudget::retaining();
        retaining.nodes = MAX_BACKEND_WALK_NODES;
        streaming.nodes = MAX_BACKEND_WALK_NODES;
        assert!(streaming.record("/one-more", 1).is_ok());
        assert!(retaining.record("/one-more", 1).is_err());
    }
}
