//! Replace a retried subtree using the measured original ancestor subtotals.
//! Missing named parents were folded into aggregates by the retention budget.
use super::{aggregate_name, is_aggregate_name, SizeNode};

pub(crate) fn merge_retry_tree(
    tree: &mut SizeNode,
    segments: &[&str],
    replacement: SizeNode,
    previous_sizes: &[u64],
) -> bool {
    if segments.is_empty() {
        *tree = replacement;
        return true;
    }
    if previous_sizes.len() != segments.len() {
        return false;
    }
    let Some((first, rest)) = segments.split_first() else {
        return false;
    };
    if let Some(index) = tree
        .children
        .iter()
        .position(|child| child.name.as_ref() == *first)
    {
        let old = tree.children[index].size;
        if !merge_retry_tree(
            &mut tree.children[index],
            rest,
            replacement,
            &previous_sizes[1..],
        ) {
            return false;
        }
        tree.size = tree
            .size
            .saturating_sub(old)
            .saturating_add(tree.children[index].size);
    } else {
        if previous_sizes.windows(2).any(|pair| pair[0] < pair[1]) {
            return false;
        }
        let previous_branch = previous_sizes[0];
        let aggregate_total = tree
            .children
            .iter()
            .filter(|child| is_aggregate_name(&child.name))
            .fold(0u64, |sum, child| sum.saturating_add(child.size));
        if aggregate_total < previous_branch {
            return false;
        }
        let mut branch = replacement;
        // Restore the old, still valid remainder of every collapsed parent
        // beneath its own path; only the retried child's contribution changes.
        for index in (0..segments.len() - 1).rev() {
            let remainder = previous_sizes[index] - previous_sizes[index + 1];
            let size = remainder.saturating_add(branch.size);
            let mut children = vec![branch];
            if remainder > 0 {
                children.push(SizeNode {
                    name: aggregate_name(1).into_boxed_str(),
                    size: remainder,
                    is_dir: false,
                    children: Vec::new(),
                });
            }
            branch = SizeNode {
                name: segments[index].into(),
                size,
                is_dir: true,
                children,
            };
        }
        let mut remove = previous_branch;
        for child in tree
            .children
            .iter_mut()
            .filter(|child| is_aggregate_name(&child.name))
        {
            let amount = child.size.min(remove);
            child.size -= amount;
            remove -= amount;
        }
        tree.size = tree
            .size
            .saturating_sub(previous_branch)
            .saturating_add(branch.size);
        tree.children.push(branch);
    }
    tree.children
        .sort_by_key(|child| std::cmp::Reverse(child.size));
    true
}

pub(crate) struct RetryOmission {
    pub segments: Vec<String>,
    pub sizes: Vec<u64>,
}

/// Recover the previously known facts for scopes which the new walk omitted.
/// A missing name inside an old aggregate has no separately provable subtotal;
/// refuse that merge rather than pretending those old bytes disappeared.
pub(crate) fn preserve_retry_omissions(
    previous: &SizeNode,
    replacement: &mut SizeNode,
    omissions: &[RetryOmission],
) -> Option<Vec<(Vec<String>, u64, u64)>> {
    let mut restored = Vec::new();
    for omission in omissions {
        let segments: Vec<_> = omission.segments.iter().map(String::as_str).collect();
        let Some(old) = known_node(previous, &segments)? else {
            continue;
        };
        let before = omission.sizes.last().copied().unwrap_or(replacement.size);
        let after = old.size;
        if !merge_retry_tree(replacement, &segments, clone_node(old), &omission.sizes) {
            return None;
        }
        restored.push((omission.segments.clone(), before, after));
    }
    Some(restored)
}

fn known_node<'a>(node: &'a SizeNode, segments: &[&str]) -> Option<Option<&'a SizeNode>> {
    let Some((first, rest)) = segments.split_first() else {
        return Some(Some(node));
    };
    match node
        .children
        .iter()
        .find(|child| child.name.as_ref() == *first)
    {
        Some(child) => known_node(child, rest),
        None if node
            .children
            .iter()
            .any(|child| is_aggregate_name(&child.name)) =>
        {
            None
        }
        None => Some(None),
    }
}

pub(crate) fn retry_node<'a>(node: &'a SizeNode, segments: &[&str]) -> Option<&'a SizeNode> {
    known_node(node, segments).flatten()
}

fn clone_node(node: &SizeNode) -> SizeNode {
    SizeNode {
        name: node.name.clone(),
        size: node.size,
        is_dir: node.is_dir,
        children: node.children.iter().map(clone_node).collect(),
    }
}
