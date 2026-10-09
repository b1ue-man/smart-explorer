//! Whether two observations of a remote file describe the same remote state
//! (the conflict checks around a mounted save). Providers report some facts
//! only in some answers: a cached listing may lack the item id or content
//! hash that a single stat carries, and second-precision listings truncate
//! the modification time. A fact missing on one side is unknown, not
//! different; a known difference in any fact is a change.
use super::types::Baseline;

impl Baseline {
    pub(super) fn same_remote_state(&self, other: &Baseline) -> bool {
        match (self, other) {
            (Baseline::Missing, Baseline::Missing) => true,
            (
                Baseline::Present {
                    id,
                    size,
                    mtime_ms,
                    content_md5,
                },
                Baseline::Present {
                    id: other_id,
                    size: other_size,
                    mtime_ms: other_mtime_ms,
                    content_md5: other_content_md5,
                },
            ) => {
                size == other_size
                    && same_mtime(*mtime_ms, *other_mtime_ms)
                    && known_equal(id, other_id)
                    && known_equal(content_md5, other_content_md5)
            }
            _ => false,
        }
    }
}

fn known_equal(left: &Option<String>, right: &Option<String>) -> bool {
    match (left, right) {
        (Some(left), Some(right)) => left == right,
        _ => true,
    }
}

/// Equal, or one side is the other truncated to whole seconds.
fn same_mtime(left: i64, right: i64) -> bool {
    left == right
        || (left % 1000 == 0 && right.div_euclid(1000) * 1000 == left)
        || (right % 1000 == 0 && left.div_euclid(1000) * 1000 == right)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn present(id: Option<&str>, size: u64, mtime_ms: i64, md5: Option<&str>) -> Baseline {
        Baseline::Present {
            id: id.map(str::to_string),
            size,
            mtime_ms,
            content_md5: md5.map(str::to_string),
        }
    }

    #[test]
    fn mount_save_task_facts_known_on_one_side_only_are_not_a_change() {
        let stat = present(Some("id-1"), 10, 1_700_000_000_123, Some("abc"));
        let listing = present(None, 10, 1_700_000_000_123, None);
        assert!(stat.same_remote_state(&listing));
        assert!(listing.same_remote_state(&stat));
        let truncated = present(Some("id-1"), 10, 1_700_000_000_000, Some("abc"));
        assert!(stat.same_remote_state(&truncated));
        assert!(Baseline::Missing.same_remote_state(&Baseline::Missing));
    }

    #[test]
    fn mount_save_task_every_known_difference_is_a_change() {
        let base = present(Some("id-1"), 10, 1_700_000_000_123, Some("abc"));
        assert!(!base.same_remote_state(&present(
            Some("id-2"),
            10,
            1_700_000_000_123,
            Some("abc")
        )));
        assert!(!base.same_remote_state(&present(
            Some("id-1"),
            11,
            1_700_000_000_123,
            Some("abc")
        )));
        assert!(!base.same_remote_state(&present(
            Some("id-1"),
            10,
            1_700_000_001_123,
            Some("abc")
        )));
        assert!(!base.same_remote_state(&present(
            Some("id-1"),
            10,
            1_700_000_000_123,
            Some("abd")
        )));
        assert!(!base.same_remote_state(&present(None, 10, 1_700_000_000_500, None)));
        assert!(!base.same_remote_state(&Baseline::Missing));
        assert!(!Baseline::Missing.same_remote_state(&base));
    }
}
