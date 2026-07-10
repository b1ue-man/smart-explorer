use std::io;

const MAX_BACKEND_WALK_NODES: u64 = 1_000_000;
const MAX_BACKEND_WALK_TEXT_BYTES: u64 = 128 * 1024 * 1024;
const MAX_BACKEND_WALK_DEPTH: usize = 512;

#[derive(Default)]
pub(super) struct WalkBudget {
    nodes: u64,
    text_bytes: u64,
}

impl WalkBudget {
    pub(super) fn record(&mut self, path: &str, depth: usize) -> io::Result<()> {
        if depth > MAX_BACKEND_WALK_DEPTH {
            return Err(invalid(format!(
                "backend walk exceeds {MAX_BACKEND_WALK_DEPTH} levels"
            )));
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
        let mut budget = WalkBudget::default();
        assert_eq!(
            budget
                .record("deep", MAX_BACKEND_WALK_DEPTH + 1)
                .unwrap_err()
                .kind(),
            io::ErrorKind::InvalidData
        );
    }
}
