//! Final retention bound, including aggregate nodes and synthetic export roots.
use super::{aggregate_name, Progress, SizeNode};

/// Keep exact subtree sizes while fitting the receiver's node budget. The
/// scanner's shared budget bounds the initial tree; this pass accounts for
/// aggregate nodes and container roots, which also consume receiver memory.
pub(crate) fn fit_tree(node: &mut SizeNode, limit: u64, progress: &Progress) -> std::io::Result<u64> {
    progress.check_cancel()?;
    if limit == 0 || (limit == 1 && !node.children.is_empty()) {
        return Err(std::io::Error::new(std::io::ErrorKind::InvalidInput, "Knotenbudget kann Wurzel und Aggregat nicht halten"));
    }
    if node.children.is_empty() { return Ok(1); }
    let children = std::mem::take(&mut node.children);
    let total = children.len();
    let mut used = 1u64;
    let mut collapsed = (0u64, 0u64);
    for (index, mut child) in children.into_iter().enumerate() {
        progress.check_cancel()?;
        let reserve = u64::from(index + 1 < total || collapsed.1 != 0);
        let remaining = limit.saturating_sub(used).saturating_sub(reserve);
        let minimum = if child.children.is_empty() { 1 } else { 2 };
        if remaining < minimum {
            collapsed.0 = collapsed.0.saturating_add(child.size);
            collapsed.1 += 1;
            continue;
        }
        used += fit_tree(&mut child, remaining, progress)?;
        node.children.push(child);
    }
    if collapsed.1 != 0 {
        node.children.push(SizeNode {
            name: aggregate_name(collapsed.1).into_boxed_str(),
            size: collapsed.0,
            is_dir: false,
            children: Vec::new(),
        });
        used += 1;
    }
    Ok(used)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn review_task_receiver_budget_includes_aggregate_nodes() -> std::io::Result<()> {
        let mut root = SizeNode {
            name: "/".into(), size: 100, is_dir: true,
            children: (0..100).map(|index| SizeNode {
                name: format!("file-{index}").into_boxed_str(), size: 1,
                is_dir: false, children: Vec::new(),
            }).collect(),
        };
        let progress = Progress::default();
        assert_eq!(fit_tree(&mut root, 4, &progress)?, 4);
        assert_eq!(root.children.iter().map(|child| child.size).sum::<u64>(), 100);
        assert_eq!(super::super::tree_transfer::shape(Some(&root), &progress)?.nodes, 4);
        Ok(())
    }
}
