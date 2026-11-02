use std::cmp::Ordering;

use super::types::{DuplicateGroup, ReclaimItem};

/// Retain only the `limit` best values. `compare(a, b) == Less` means `a`
/// should be presented before `b`. Values stay presentation-sorted, making a
/// rejected candidate O(log limit) and never temporarily retaining limit + 1.
pub(super) fn retain_best<T>(
    values: &mut Vec<T>,
    value: T,
    limit: usize,
    compare: impl Fn(&T, &T) -> Ordering,
) {
    if limit == 0 {
        return;
    }
    let insert_at = values
        .binary_search_by(|probe| compare(probe, &value))
        .unwrap_or_else(|index| index);
    if values.len() < limit {
        values.insert(insert_at, value);
        return;
    }
    if insert_at < limit {
        values.pop();
        values.insert(insert_at, value);
    }
}

pub(super) fn compare_item_size(left: &ReclaimItem, right: &ReclaimItem) -> Ordering {
    right
        .size
        .cmp(&left.size)
        .then_with(|| left.path.cmp(&right.path))
}

pub(super) fn compare_item_path(left: &ReclaimItem, right: &ReclaimItem) -> Ordering {
    left.path.cmp(&right.path)
}

pub(super) fn compare_group(left: &DuplicateGroup, right: &DuplicateGroup) -> Ordering {
    right
        .reclaimable
        .cmp(&left.reclaimable)
        .then_with(|| right.size.cmp(&left.size))
        .then_with(|| left.hash.hex.cmp(&right.hash.hex))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn collector_keeps_deterministic_best_values_at_the_bound() {
        let mut values = Vec::new();
        for value in [4, 1, 3, 2, 5] {
            retain_best(&mut values, value, 3, i32::cmp);
        }
        assert_eq!(values, vec![1, 2, 3]);

        let mut none = Vec::new();
        retain_best(&mut none, 1, 0, i32::cmp);
        assert!(none.is_empty());
    }
}
