//! Which session decides the size of the virtual display (P5, E2): the
//! session with control, else the newest view session; after the last
//! session the size stays; at least `MIN_RESIZE_GAP_MS` between two
//! changes. A session with a fixed size („Auflösung“ of the viewer) keeps
//! it: its window sizes do not count. Pure bookkeeping with explicit times,
//! so the rules are tested without a clock (`virtual_display_host.rs`
//! drives it).
use super::{fit_mode, DriverLimit, FrameSize, MIN_RESIZE_GAP_MS};

/// The size one session asks for.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Claim {
    /// The registry id of the session.
    pub session: String,
    /// The session holds control (`SessionMode::Control`).
    pub control: bool,
    /// Order of arrival: a higher one is newer.
    pub order: u64,
    /// The viewer's display size (hello), its latest `SET_MODE` size, or
    /// its fixed size.
    pub size: FrameSize,
    /// The viewer fixed the size; `SET_MODE` sizes leave it.
    pub fixed: bool,
}

/// The claim that decides: the newest one with control, else the newest.
pub fn owner(claims: &[Claim]) -> Option<&Claim> {
    claims
        .iter()
        .filter(|claim| claim.control)
        .max_by_key(|claim| claim.order)
        .or_else(|| claims.iter().max_by_key(|claim| claim.order))
}

/// What the display should do now.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Due {
    /// Nothing: no claim, or the display has the size already.
    Idle,
    /// Change to this size now.
    Now(FrameSize),
    /// A change is due after this many milliseconds (the gap since the last).
    Later(u64),
}

/// The claims of the running sessions and the last change.
#[derive(Clone, Debug, Default)]
pub struct Planner {
    claims: Vec<Claim>,
    next_order: u64,
    applied: Option<FrameSize>,
    last_change_ms: Option<u64>,
}

impl Planner {
    /// A session asks for `size` (a new session, or a new size of one).
    /// A new size of a known session keeps its order: a resize does not
    /// make an older session the newest.
    pub fn claim(&mut self, session: &str, control: bool, size: FrameSize, fixed: bool) {
        if let Some(known) = self
            .claims
            .iter_mut()
            .find(|claim| claim.session == session)
        {
            known.control = control;
            known.size = size;
            known.fixed = fixed;
            return;
        }
        self.next_order += 1;
        self.claims.push(Claim {
            session: session.to_string(),
            control,
            order: self.next_order,
            size,
            fixed,
        });
    }

    /// A known session's window has `size` now (`SET_MODE`); `false` for an
    /// unknown session and a session with a fixed size.
    pub fn resize(&mut self, session: &str, size: FrameSize) -> bool {
        match self
            .claims
            .iter_mut()
            .find(|claim| claim.session == session && !claim.fixed)
        {
            Some(known) => {
                known.size = size;
                true
            }
            None => false,
        }
    }

    /// A session ended; the display keeps its size.
    pub fn release(&mut self, session: &str) {
        self.claims.retain(|claim| claim.session != session);
    }

    pub fn has_claims(&self) -> bool {
        !self.claims.is_empty()
    }

    /// The fitted size the deciding session asks for.
    pub fn target(&self, limit: DriverLimit) -> Option<FrameSize> {
        owner(&self.claims).and_then(|claim| fit_mode(claim.size, limit))
    }

    /// The size se asked the display for last (`changed`).
    pub fn applied(&self) -> Option<FrameSize> {
        self.applied
    }

    /// What is due at `now_ms` within `limit`.
    pub fn due(&self, limit: DriverLimit, now_ms: u64) -> Due {
        let Some(target) = self.target(limit) else {
            return Due::Idle;
        };
        if self.applied == Some(target) {
            return Due::Idle;
        }
        let ready = self
            .last_change_ms
            .map_or(0, |last| last.saturating_add(MIN_RESIZE_GAP_MS));
        if now_ms >= ready {
            Due::Now(target)
        } else {
            Due::Later(ready - now_ms)
        }
    }

    /// The display got another size than se's (a physical monitor's own
    /// mode back): the next claim is applied even with the last size.
    pub fn forget(&mut self) {
        self.applied = None;
    }

    /// se asked the display for `attempted` at `now_ms`. A failed or
    /// narrowed attempt counts too: the same size is not asked again until
    /// the deciding session wants another one, and the gap applies to
    /// failures as well, so a failing driver is not hammered.
    pub fn changed(&mut self, attempted: FrameSize, now_ms: u64) {
        self.last_change_ms = Some(now_ms);
        self.applied = Some(attempted);
    }
}
