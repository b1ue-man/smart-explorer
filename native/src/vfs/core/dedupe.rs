//! Default execution of an already authorized exact-ID duplicate plan.
use super::{Backend, DedupeCandidate, VfsResult};
use std::io;

pub(super) fn apply<B: Backend + ?Sized>(backend: &B, plan: &[DedupeCandidate]) -> VfsResult<usize> {
        let mut removed = 0usize;
        for candidate in plan {
            if let Err(error) = backend.remove_file_id(&candidate.path, candidate.id.as_deref()) {
                return Err(io::Error::new(
                    error.kind(),
                    format!(
                        "duplicate cleanup stopped after {removed}/{} exact removals at {} (id {:?}): {error}",
                        plan.len(), candidate.path, candidate.id
                    ),
                ));
            }
            removed += 1;
        }
        Ok(removed)
}
