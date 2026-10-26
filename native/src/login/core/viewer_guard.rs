//! A drawn image, its complete signed context and its bounds authorize local input.
use super::{
    gateway_types::GatewayAccess,
    model::{Challenge, SessionBinding},
};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{collections::VecDeque, io};
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct FrameToken {
    pub frame_id: u64,
    pub width: u32,
    pub height: u32,
    pub digest: String,
}
#[derive(Clone, PartialEq, Eq, Serialize)]
struct Context {
    machine: String,
    key: String,
    boot: String,
    binding: SessionBinding,
    policy: u64,
    peer: String,
    peer_key: String,
    target_user: String,
}
impl Context {
    fn new(value: &Challenge, access: &GatewayAccess) -> Self {
        Self {
            machine: value.machine_id.clone(),
            key: value.machine_public_key.clone(),
            boot: value.boot_generation.clone(),
            binding: value.binding.clone(),
            policy: value.policy_revision,
            peer: access.peer_id.clone(),
            peer_key: access.public_key.clone(),
            target_user: access.target_user.clone(),
        }
    }
}
#[derive(Default)]
pub(crate) struct ViewerGuard {
    context: Option<Context>,
    pub latest: Option<FrameToken>,
    previous: Option<FrameToken>,
    pub presented: Option<FrameToken>,
    pub focused: bool,
    pub control: bool,
    pub closed: bool,
    pub epoch: u64,
    shown: VecDeque<FrameToken>,
}
impl ViewerGuard {
    pub fn adopt(&mut self, challenge: &Challenge, access: &GatewayAccess) -> io::Result<()> {
        let actual = Context::new(challenge, access);
        if self.context.as_ref().is_some_and(|old| old != &actual) {
            self.close();
            return Err(io::Error::other(
                "login boot, session, desktop or policy generation changed",
            ));
        }
        self.context = Some(actual);
        Ok(())
    }
    pub fn frame(&mut self, frame_id: u64, width: u32, height: u32) -> io::Result<FrameToken> {
        if self.closed {
            return Err(io::Error::other("login presentation already ended"));
        }
        crate::screen::core::pixel_convert::FrameSize::new(width, height).check_frame()?;
        let context = self
            .context
            .as_ref()
            .ok_or_else(|| io::Error::other("login frame has no signed context"))?;
        #[derive(Serialize)]
        struct Body<'a> {
            domain: &'static str,
            context: &'a Context,
            frame_id: u64,
            width: u32,
            height: u32,
        }
        let bytes = serde_json::to_vec(&Body {
            domain: "smart-explorer/login-presented/v1",
            context,
            frame_id,
            width,
            height,
        })
        .map_err(io::Error::other)?;
        let digest = Sha256::digest(bytes)
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect();
        let token = FrameToken {
            frame_id,
            width,
            height,
            digest,
        };
        if self
            .latest
            .as_ref()
            .is_some_and(|old| old.width != width || old.height != height)
        {
            self.invalidate();
        }
        self.previous = self.latest.take();
        self.latest = Some(token.clone());
        Ok(token)
    }
    pub fn presented(&mut self, token: &FrameToken, epoch: u64) -> bool {
        if epoch != self.epoch
            || self.closed
            || !self.same_bounds(token)
            || (self.latest.as_ref() != Some(token)
                && self.previous.as_ref() != Some(token)
                && !self.shown.contains(token))
        {
            return false;
        }
        self.presented = Some(token.clone());
        if self.shown.back() != Some(token) {
            self.shown.push_back(token.clone());
        }
        // The owner's queue holds eight input batches plus the admitted batch.
        // Retain their actual draw proofs while newer same-context pixels arrive.
        while self.shown.len() > 9 {
            self.shown.pop_front();
        }
        true
    }
    pub fn action_ready(&self, token: &FrameToken) -> bool {
        !self.closed && self.same_bounds(token) && self.shown.contains(token)
    }
    pub fn input_ready(&self, token: &FrameToken) -> bool {
        self.focused && self.control && self.action_ready(token)
    }
    fn same_bounds(&self, token: &FrameToken) -> bool {
        self.latest
            .as_ref()
            .is_some_and(|latest| latest.width == token.width && latest.height == token.height)
    }
    pub fn invalidate(&mut self) {
        self.presented = None;
        self.shown.clear();
        if let Some(next) = self.epoch.checked_add(1) {
            self.epoch = next;
        } else {
            self.closed = true;
        }
    }
    pub fn close(&mut self) {
        self.closed = true;
        self.invalidate();
        self.latest = None;
        self.previous = None;
    }
}
