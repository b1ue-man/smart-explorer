//! A popup's input target and its WM activation anchor are distinct identities.
use super::*;

pub(super) fn owner_focus(kind: WindowKind, override_redirect: bool) -> bool {
    kind == WindowKind::Menu || (kind == WindowKind::Popup && override_redirect)
}

pub(super) fn can_activate(kind: WindowKind, override_redirect: bool) -> bool {
    !override_redirect && !matches!(kind, WindowKind::Menu | WindowKind::Tooltip)
}

#[derive(Clone)]
pub(super) struct Node {
    pub binding: WindowBinding,
    /// The freshly verified XRes client + process start/session/user signature.
    pub process: Vec<u8>,
    pub owner: Option<Window>,
    pub owner_focus: bool,
    pub eligible: bool,
}

fn anchor(
    mut node: Node,
    read: &mut impl FnMut(Window) -> io::Result<Option<Node>>,
) -> io::Result<Option<Window>> {
    let process = node.process.clone();
    let mut visited = HashSet::new();
    loop {
        let window = xid(node.binding.native)?;
        if !node.eligible || node.process != process || !visited.insert(window) {
            return Ok(None);
        }
        // A regular dialog is its own activation boundary even if it has an
        // owner. Never skip it to accept its disabled/modal ancestor instead.
        if !node.owner_focus {
            return Ok(Some(window));
        }
        let Some(owner) = node.owner else {
            return Ok(None);
        };
        let Some(parent) = read(owner)? else {
            return Ok(None);
        };
        node = parent;
    }
}

/// `read` returns only already-bound, freshly validated window lifetimes.
/// Window ancestry is used solely to find the actual focus control's toplevel;
/// every transient hop still needs an independent binding/process proof.
pub(super) fn confirmed(
    binding: &WindowBinding,
    active: Window,
    mut input: Window,
    root: Window,
    mut read: impl FnMut(Window) -> io::Result<Option<Node>>,
    mut parent: impl FnMut(Window) -> io::Result<Window>,
) -> io::Result<bool> {
    let Some(target) = read(xid(binding.native)?)? else {
        return Ok(false);
    };
    if target.binding != *binding {
        return Ok(false);
    }
    let process = target.process.clone();
    let Some(target_anchor) = anchor(target, &mut read)? else {
        return Ok(false);
    };
    if active != target_anchor {
        return Ok(false);
    }
    let mut visited = HashSet::new();
    // None and PointerRoot are not an activation proof for any family member.
    while input > 1 && input != root && visited.insert(input) {
        if input == target_anchor {
            return Ok(true);
        }
        if let Some(node) = read(input)? {
            // Toolkits may put real X focus in an override-redirect popup (or
            // its child control) while EWMH keeps the regular owner active.
            if node.owner_focus
                && node.process == process
                && anchor(node, &mut read)? == Some(target_anchor)
            {
                return Ok(true);
            }
        }
        input = parent(input)?;
    }
    Ok(false)
}
