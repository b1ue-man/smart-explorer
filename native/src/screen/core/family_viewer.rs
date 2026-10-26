//! Shared desktop/phone viewer gate. Rendering still owns PRESENTED.
use std::io;
use crate::remote_window::{FamilySnapshot, OperationStatus, WindowIdentity, WindowInputTarget, WindowOperation,WindowKind};
use super::family_protocol::{selector, token, FamilyInput, FamilyListEntry, FamilyOperation, FamilyOperationResult};
use super::input_model::{InputEvent, MAX_INPUT_EVENTS};
use super::window_list::SourceId;
use super::wire_control::ViewerFrame;

#[derive(Clone, Debug)]
struct PendingFocus {
    request: FamilyOperation,
    layout: u64,
    target: WindowIdentity,
    events: Vec<InputEvent>,
}

#[derive(Clone, Debug)]
pub(crate) struct FamilyViewer {
    pub expected: WindowIdentity,
    pub snapshot: Option<FamilySnapshot>,
    pub notice: Option<String>,
    pub last_operation: Option<FamilyOperationResult>,
    armed: Option<WindowIdentity>,
    pointer: Option<WindowIdentity>,
    pending: Option<PendingFocus>,
}

impl FamilyViewer {
    pub(crate) fn for_source(source: &SourceId) -> Option<Self> {
        Some(Self { expected: WindowIdentity { session: selector(source)?.into(), generation: 1 },
            snapshot: None, notice: None, last_operation: None, armed: None, pointer: None, pending: None })
    }
    pub(crate) fn validate_entry(entry: &FamilyListEntry) -> io::Result<()> {
        if selector(&entry.source) != Some(entry.identity.session.as_str()) || entry.identity.generation != 1 {
            return Err(io::Error::new(io::ErrorKind::InvalidData, "invalid pinned family choice"));
        }
        Ok(())
    }
    /// Returns true only when capture layout changed, not title/icon/focus metadata.
    pub(crate) fn receive(&mut self, snapshot: FamilySnapshot) -> io::Result<bool> {
        snapshot.validate()?;
        if snapshot.root != self.expected || snapshot.session != self.expected.session {
            return Err(io::Error::new(io::ErrorKind::InvalidData, "family selection identity changed"));
        }
        if let Some(old) = &self.snapshot {
            if snapshot.revision < old.revision { return Ok(false); }
            if snapshot.revision == old.revision && snapshot != *old {
                return Err(io::Error::new(io::ErrorKind::InvalidData, "conflicting family snapshot revision"));
            }
            if snapshot.layout_revision < old.layout_revision {
                return Err(io::Error::new(io::ErrorKind::InvalidData, "regressing family layout"));
            }
        }
        let changed = self.snapshot.as_ref().is_some_and(|old| old.layout_revision != snapshot.layout_revision);
        if changed { self.pending = None; self.pointer = None; self.armed = None; }
        if self.armed.as_ref().is_some_and(|id| !snapshot.members.iter().any(|m|
            &m.identity == id && m.focused && m.visible && !m.minimized && m.operations.input)) {
            self.armed = None;
        }
        self.snapshot = Some(snapshot);
        Ok(changed)
    }
    pub(crate) fn disarm(&mut self) {
        self.armed = None;
        self.pending = None;
    }
    pub(crate) fn operation(&mut self, identity: WindowIdentity, operation: WindowOperation) -> io::Result<FamilyOperation> {
        let snapshot = self.snapshot.as_ref().ok_or_else(|| io::Error::other("family snapshot pending"))?;
        let member = snapshot.members.iter().find(|m| m.identity == identity)
            .ok_or_else(|| io::Error::other("stale window generation"))?;
        let supported = match operation { WindowOperation::Focus => member.operations.focus,
            WindowOperation::Close => member.operations.close,
            WindowOperation::Resize { width, height } => member.operations.resize && width > 0 && height > 0 };
        if !supported { return Err(io::Error::new(io::ErrorKind::Unsupported, "window operation unavailable")); }
        let request = FamilyOperation { request_id: token()?, identity, operation };
        if operation == WindowOperation::Focus {
            self.armed = None;
            self.pending = Some(PendingFocus { request: request.clone(), target:request.identity.clone(),layout: snapshot.layout_revision, events: Vec::new() });
        }
        self.notice = Some("Fensteranfrage wartet auf Bestätigung.".into());
        Ok(request)
    }
    /// Reply is correlated before it can authorize input. A requested close is
    /// never interpreted as application exit; save dialogs remain in the stream.
    pub(crate) fn result(&mut self, result: FamilyOperationResult) -> io::Result<(bool, Vec<ViewerFrame>)> {
        result.validate()?;
        let focus = self.pending.as_ref().filter(|p| p.request.request_id == result.request_id).cloned();
        let changed = if let Some(snapshot) = result.snapshot.clone() { self.receive(snapshot)? } else { false };
        let mut output = Vec::new();
        if let Some(focus) = focus {
            self.pending = None;
            if result.status == Some(OperationStatus::FocusConfirmed) && !changed {
                if let Some(snapshot) = &self.snapshot {
                    if snapshot.layout_revision == focus.layout && snapshot.members.iter().any(|m|
                        m.identity == focus.request.identity && m.focused && m.visible && !m.minimized && m.operations.input) {
                        self.armed = Some(focus.request.identity.clone());
                        if !focus.events.is_empty() {
                            output.push(ViewerFrame::FamilyInput(FamilyInput { target: WindowInputTarget {
                                identity: focus.target, snapshot_revision: snapshot.revision }, events: focus.events }));
                        }
                    }
                }
            }
        }
        self.notice = Some(result.reason.clone().unwrap_or_else(|| match result.status {
            Some(OperationStatus::FocusConfirmed) => "Fensterfokus bestätigt.".into(),
            _ => "Anfrage angenommen; der aktuelle Fensterzustand folgt im Bild und Snapshot.".into(),
        }));
        self.last_operation = Some(result);
        Ok((changed, output))
    }
    /// Canvas input becomes member-normalized batches. Call only behind the
    /// actual rendered-image floor; no caller may fall back to raw INPUT.
    pub(crate) fn input(&mut self, events: Vec<InputEvent>) -> io::Result<Vec<ViewerFrame>> {
        if events.len() > MAX_INPUT_EVENTS { return Err(io::Error::other("family input batch too large")); }
        let snapshot = self.snapshot.clone().ok_or_else(|| io::Error::other("family snapshot pending"))?;
        let mut output = Vec::new();
        for event in events {
            let (identity, event) = match event {
                InputEvent::PointerAbs { x, y } => {
                    let Some((target,x,y)) = snapshot.pointer_target(x,y) else { self.pointer=None; continue; };
                    self.pointer=Some(target.identity.clone());
                    (target.identity, InputEvent::PointerAbs { x,y })
                }
                InputEvent::Touch { id,phase,x,y } => {
                    let Some((target,x,y)) = snapshot.pointer_target(x,y) else { continue; };
                    self.pointer=Some(target.identity.clone());
                    (target.identity, InputEvent::Touch { id,phase,x,y })
                }
                InputEvent::Button { .. } | InputEvent::Wheel { .. } => {
                    let Some(identity) = self.pointer.clone() else { continue; }; (identity,event)
                }
                InputEvent::Action { .. } => continue,
                _ => {
                    let Some(target) = snapshot.focused_target() else { continue; }; (target.identity,event)
                }
            };
            let focus_identity=Self::focus_identity(&snapshot,&identity).ok_or_else(||io::Error::other("window focus unavailable"))?;
            if self.armed.as_ref() == Some(&focus_identity) {
                output.push(ViewerFrame::FamilyInput(FamilyInput { target: WindowInputTarget {
                    identity, snapshot_revision:snapshot.revision },events:vec![event] }));
            } else if self.pending.as_ref().is_some_and(|p|p.target==identity && p.layout==snapshot.layout_revision) {
                if let Some(pending) = &mut self.pending {
                    if matches!(event,InputEvent::PointerAbs {..}) && pending.events.last().is_some_and(|e|matches!(e,InputEvent::PointerAbs {..})) {
                        if let Some(last)=pending.events.last_mut(){*last=event;}
                    } else if pending.events.len()<MAX_INPUT_EVENTS { pending.events.push(event); }
                    else { self.disarm(); return Err(io::Error::other("focus acknowledgement pending; input discarded")); }
                }
            } else {
                // Never infer activation from a process, owner or display label.
                let operation = self.operation(focus_identity, WindowOperation::Focus)?;
                if let Some(pending) = &mut self.pending { pending.target=identity;pending.events.push(event); }
                output.push(ViewerFrame::FamilyOperation(operation));
            }
        }
        Ok(output)
    }
    fn focus_identity(snapshot:&FamilySnapshot,target:&WindowIdentity)->Option<WindowIdentity> {
        let mut member=snapshot.members.iter().find(|m|&m.identity==target)?;
        for _ in 0..snapshot.members.len() {
            if !member.operations.input||!member.visible||member.minimized {return None;}
            if member.kind!=WindowKind::Menu&&member.operations.focus {return Some(member.identity.clone());}
            // Menus and nonactivating popups keep their own input identity,
            // but confirm their regular owner. A disabled dialog is a boundary.
            if !matches!(member.kind,WindowKind::Menu|WindowKind::Popup) {return None;}
            let owner=member.owner.as_ref()?;
            member=snapshot.members.iter().find(|m|&m.identity==owner)?;
        }
        None
    }
}
