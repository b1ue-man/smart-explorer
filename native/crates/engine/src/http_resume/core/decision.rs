//! Decisions of a resumable HTTP download (user rule 2026-11-17: a download
//! that still moves is never cut off by a fixed total time; only a real
//! stall ends it, and a broken one continues where it stopped).
//!
//! An attempt that keeps bytes asks for the rest (`Range: bytes=<kept>-`,
//! RFC 9110 §14.2, with `If-Range` and the strong entity tag of the earlier
//! answer, §13.1.5). The answer decides: a 206 whose `Content-Range` (§14.4)
//! starts exactly at the kept bytes continues them, a 200 is the whole
//! resource again and replaces them, anything else cannot be used. The
//! attempts end only when several in a row brought no new byte and no new
//! byte arrived for a whole stall window.
use std::time::{Duration, Instant};

/// `Content-Range: bytes <first>-<last>/<complete length or *>`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct ContentRange {
    pub(crate) first: u64,
    pub(crate) last: u64,
    pub(crate) complete: Option<u64>,
}

fn digits(text: &str) -> Option<u64> {
    let text = text.trim();
    if text.is_empty() || !text.bytes().all(|byte| byte.is_ascii_digit()) {
        return None;
    }
    text.parse().ok()
}

/// A satisfied byte range; `None` for every other form (another unit, the
/// unsatisfied `bytes */<length>`, a last byte before the first or beyond
/// the complete length).
pub(crate) fn parse_content_range(value: &str) -> Option<ContentRange> {
    let (unit, rest) = value.trim().split_once(' ')?;
    if !unit.eq_ignore_ascii_case("bytes") {
        return None;
    }
    let (range, complete) = rest.split_once('/')?;
    let complete = match complete.trim() {
        "*" => None,
        length => Some(digits(length)?),
    };
    let (first, last) = range.split_once('-')?;
    let (first, last) = (digits(first)?, digits(last)?);
    if last < first || complete.is_some_and(|complete| last >= complete) {
        return None;
    }
    Some(ContentRange {
        first,
        last,
        complete,
    })
}

/// The `Range` header of an attempt that keeps `kept` bytes; none for the
/// first attempt.
pub(crate) fn range_header(kept: u64) -> Option<String> {
    (kept > 0).then(|| format!("bytes={kept}-"))
}

/// The `If-Range` validator of an answer: only a strong entity tag (a weak
/// one, `W/"…"`, must not be used there, RFC 9110 §13.1.5).
pub(crate) fn if_range_validator(etag: Option<&str>) -> Option<&str> {
    let tag = etag?.trim();
    (tag.len() >= 2 && tag.starts_with('"') && tag.ends_with('"')).then_some(tag)
}

/// What is known before an attempt.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct Kept {
    /// Bytes kept from earlier attempts.
    pub(crate) offset: u64,
    /// The exact length a signature or a pin names.
    pub(crate) pinned: Option<u64>,
    /// The length an earlier answer named.
    pub(crate) known: Option<u64>,
}

/// The facts of one answer.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct Reply<'a> {
    pub(crate) status: u16,
    pub(crate) content_range: Option<&'a str>,
    pub(crate) content_length: Option<u64>,
}

/// What an answer means for the kept bytes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Answer {
    /// The body continues the kept bytes; the resource has `total` bytes.
    Continue { total: Option<u64> },
    /// The body is the whole resource from byte 0; the kept bytes go.
    Restart { total: Option<u64> },
    /// The answer names another length than the pinned one: no attempt can
    /// change that, the download fails.
    WrongLength { announced: u64, pinned: u64 },
    /// The answer cannot be used; the kept bytes go and another attempt
    /// starts from byte 0.
    Unusable,
}

/// What the answer `reply` to an attempt that keeps `kept` means.
pub(crate) fn classify(kept: Kept, reply: Reply<'_>) -> Answer {
    let wrong = |announced: Option<u64>| match (announced, kept.pinned) {
        (Some(announced), Some(pinned)) if announced != pinned => {
            Some(Answer::WrongLength { announced, pinned })
        }
        _ => None,
    };
    match reply.status {
        200 => wrong(reply.content_length).unwrap_or(Answer::Restart {
            total: kept.pinned.or(reply.content_length),
        }),
        206 => {
            let Some(range) = reply.content_range.and_then(parse_content_range) else {
                return Answer::Unusable;
            };
            let length = (range.last - range.first).saturating_add(1);
            if range.first != kept.offset || reply.content_length.is_some_and(|l| l != length) {
                return Answer::Unusable;
            }
            if let Some(answer) = wrong(range.complete) {
                return answer;
            }
            // The resource changed since the kept bytes arrived.
            if kept.offset > 0
                && matches!((kept.known, range.complete), (Some(known), Some(complete)) if known != complete)
            {
                return Answer::Unusable;
            }
            Answer::Continue {
                total: kept.pinned.or(range.complete).or(kept.known),
            }
        }
        _ => Answer::Unusable,
    }
}

/// What an error status means.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Refusal {
    /// The kept bytes lie beyond the resource (416): start again from 0.
    Restart,
    /// The server may answer later (408, 425, 429, 500, 502, 503, 504).
    Wait,
    /// Another attempt gets the same answer (404, 403, …).
    Final,
}

pub(crate) fn refusal(status: u16) -> Refusal {
    match status {
        416 => Refusal::Restart,
        408 | 425 | 429 | 500 | 502 | 503 | 504 => Refusal::Wait,
        _ => Refusal::Final,
    }
}

/// How long a download without a new byte goes on trying.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Patience {
    /// Attempts in a row without a new byte before the download may end.
    pub(crate) idle_attempts: u32,
    /// And at least this long without a new byte.
    pub(crate) stall_window: Duration,
    /// The pause after the first attempt without a new byte; it doubles.
    pub(crate) first_pause: Duration,
    pub(crate) max_pause: Duration,
}

impl Patience {
    /// Five attempts and two minutes without a new byte; pauses of 1, 2, 4,
    /// 8, 16 and then 30 s between attempts without one, none after an
    /// attempt that brought bytes.
    pub(crate) const STANDARD: Patience = Patience {
        idle_attempts: 5,
        stall_window: Duration::from_secs(120),
        first_pause: Duration::from_secs(1),
        max_pause: Duration::from_secs(30),
    };
}

/// What follows an attempt that ended short of the whole resource.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Next {
    /// Another attempt after this pause.
    Retry(Duration),
    /// No new byte for `Patience::idle_attempts` attempts and the stall
    /// window: the download ends.
    Stalled,
}

/// The attempts of one download: the most bytes ever kept and since when no
/// attempt went beyond them. Bytes a restarted answer brings again do not
/// count as progress, so a server that always breaks at the same byte ends.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Attempts {
    furthest: u64,
    idle: u32,
    last_progress: Instant,
}

impl Attempts {
    pub(crate) fn new(now: Instant) -> Self {
        Self {
            furthest: 0,
            idle: 0,
            last_progress: now,
        }
    }

    /// Attempts in a row without a new byte.
    pub(crate) fn idle(&self) -> u32 {
        self.idle
    }

    /// An attempt ended short with `kept` bytes at `now`.
    pub(crate) fn ended(&mut self, kept: u64, now: Instant, patience: &Patience) -> Next {
        if kept > self.furthest {
            self.furthest = kept;
            self.idle = 0;
            self.last_progress = now;
            return Next::Retry(Duration::ZERO);
        }
        self.idle = self.idle.saturating_add(1);
        if self.idle >= patience.idle_attempts
            && now.saturating_duration_since(self.last_progress) >= patience.stall_window
        {
            return Next::Stalled;
        }
        let doubling = self.idle.saturating_sub(1).min(16);
        Next::Retry(
            patience
                .first_pause
                .saturating_mul(1u32 << doubling)
                .min(patience.max_pause),
        )
    }
}

#[cfg(test)]
#[path = "decision_tests.rs"]
mod tests;
