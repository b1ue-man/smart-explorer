//! Bounded JNI pull transport, backpressure and completion on the same queue.
use super::*;

impl PullQueue {
    pub(super) fn set_frame_limits(
        &self,
        limits: crate::screen::core::frame_budget::FrameLimits,
        size: FrameSize,
        contract: Option<crate::screen::core::frame_budget::FrameContract>,
        admitted: bool,
    ) -> std::io::Result<()> {
        limits.check(size)?;
        let admitted = admitted
            && (contract.is_some()
                || limits.working_bytes < crate::screen::core::frame_budget::MAX_WORKING_BYTES);
        let budget = crate::screen::core::frame_size_caps::PacketBudget::for_receive(
            contract,
            admitted.then_some(limits),
        )?;
        let bound = budget.encoded_bytes;
        let bound = if contract.is_none() && !admitted {
            bound.max(64 * 1024 * 1024)
        } else {
            bound
        };
        self.frame_bytes
            .store(bound, std::sync::atomic::Ordering::Release);
        self.frame_items.store(
            if contract.is_some() || admitted {
                budget.records
            } else {
                0
            },
            std::sync::atomic::Ordering::Release,
        );
        Ok(())
    }
    /// Queues a picture; `now_us` is the clock of the stall rule.
    pub(crate) fn push(&self, pts_us: u64, mut flags: u32, data: Vec<u8>, now_us: u64) -> Pushed {
        let mut state = lock(&self.state);
        let none = |stalled| Pushed {
            queued: false,
            stalled,
        };
        if state.ended {
            return none(false);
        }
        let tiles = flags & FLAG_TILES_BGRA != 0;
        let mut stalled = false;
        let configured = self.frame_bytes.load(std::sync::atomic::Ordering::Acquire);
        let bound = if configured == 0 {
            64 * 1024 * 1024
        } else {
            configured
        };
        let slots = self.frame_items.load(std::sync::atomic::Ordering::Acquire);
        let slots = if slots == 0 { usize::MAX } else { slots };
        if data.len() > bound {
            state.clear();
            state.waiting_keyframe = true;
            return none(true);
        }
        if flags & FLAG_NEW_STREAM != 0 {
            state.clear();
        }
        let started = Instant::now();
        // Backpressure preserves a complete lossless transaction when the app
        // consumes its dirty records more slowly than the network receives them.
        while !tiles
            && (state.items.len() >= slots || state.bytes.saturating_add(data.len()) > bound)
        {
            let elapsed = u64::try_from(started.elapsed().as_micros()).unwrap_or(u64::MAX);
            let age = state.items.front().map_or(0, |front| {
                now_us
                    .saturating_add(elapsed)
                    .saturating_sub(front.queued_us)
            });
            if age >= STALL_US {
                break;
            }
            state = self
                .ready
                .wait_timeout(state, Duration::from_micros(STALL_US - age))
                .unwrap_or_else(PoisonError::into_inner)
                .0;
            if state.ended {
                return none(false);
            }
        }
        let now_us =
            now_us.saturating_add(u64::try_from(started.elapsed().as_micros()).unwrap_or(u64::MAX));
        if (!tiles
            && (state.items.len() >= slots || state.bytes.saturating_add(data.len()) > bound))
            || (!tiles
                && state
                    .items
                    .front()
                    .is_some_and(|front| now_us.saturating_sub(front.queued_us) > STALL_US))
        {
            state.clear();
            state.waiting_keyframe = true;
            stalled = true;
        }
        if flags & FLAG_NEW_STREAM != 0 {
            state.clear();
        }
        let begins = flags & FLAG_HYBRID == 0 || flags & FLAG_HYBRID_BEGIN != 0;
        if flags & FLAG_KEYFRAME != 0 && begins {
            state.waiting_keyframe = false;
        } else if !tiles && state.waiting_keyframe {
            return none(stalled);
        }
        if tiles {
            // Only the newest tile picture is kept; it keeps the mark of the
            // group start the app has not seen yet.
            let mut inherited = 0;
            let mut removed = 0usize;
            state.items.retain(|old| {
                if old.flags & FLAG_TILES_BGRA == 0 {
                    return true;
                }
                inherited |= old.flags & FLAG_NEW_STREAM;
                removed = removed.saturating_add(old.data.len());
                false
            });
            state.bytes = state.bytes.saturating_sub(removed);
            flags |= inherited;
        }
        state.bytes = state.bytes.saturating_add(data.len());
        state.items.push_back(Queued {
            pts_us,
            flags,
            data,
            queued_us: now_us,
        });
        self.ready.notify_one();
        Pushed {
            queued: true,
            stalled,
        }
    }

    /// Writes the oldest picture into `dst` (layout above) and returns the
    /// bytes written, or one of the `PULL_*` codes. Too small a `dst` leaves
    /// the picture queued and, when it holds 16 bytes, names the payload
    /// length in bytes 12–15.
    pub(crate) fn pull_into(&self, dst: &mut [u8], timeout: Duration) -> i32 {
        let deadline = Instant::now() + timeout.min(MAX_PULL_WAIT);
        let mut state = lock(&self.state);
        loop {
            if let Some(front) = state.items.front() {
                let len = front.data.len();
                let (Ok(wire_len), Ok(total)) = (
                    u32::try_from(len),
                    i32::try_from(len.saturating_add(HEADER_BYTES)),
                ) else {
                    return PULL_BRIDGE_ERROR;
                };
                if dst.len() < HEADER_BYTES + len {
                    if dst.len() >= HEADER_BYTES {
                        dst[12..16].copy_from_slice(&wire_len.to_le_bytes());
                    }
                    return PULL_TOO_SMALL;
                }
                dst[0..8].copy_from_slice(&front.pts_us.to_le_bytes());
                dst[8..12].copy_from_slice(&front.flags.to_le_bytes());
                dst[12..16].copy_from_slice(&wire_len.to_le_bytes());
                dst[HEADER_BYTES..HEADER_BYTES + len].copy_from_slice(&front.data);
                state.items.pop_front();
                state.bytes = state.bytes.saturating_sub(len);
                self.ready.notify_all();
                return total;
            }
            if state.ended {
                return PULL_ENDED;
            }
            let now = Instant::now();
            if now >= deadline {
                return PULL_TIMEOUT;
            }
            state = self
                .ready
                .wait_timeout(state, deadline - now)
                .unwrap_or_else(PoisonError::into_inner)
                .0;
        }
    }

    /// Ends the queue: it is emptied and every pull answers `PULL_ENDED`.
    pub(crate) fn close(&self) {
        let mut state = lock(&self.state);
        state.ended = true;
        state.clear();
        self.ready.notify_all();
    }

    #[cfg(test)]
    pub(crate) fn len(&self) -> usize {
        lock(&self.state).items.len()
    }
}
