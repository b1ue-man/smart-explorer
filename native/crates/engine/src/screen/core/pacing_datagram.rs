//! The pacer while the pictures go as datagrams (`video_transport.rs`,
//! „UDP“): a picture is sent and forgotten. Its acknowledgement still feeds
//! the statistics and the confirmed picture of B-36, but a picture without
//! one never resets anything: once older than the stale time it leaves the
//! count of the bytes in flight (the viewer gave it up and reports that
//! with `FRAME_LOSS`).
use super::Pacer;

impl Pacer {
    /// The pictures go as datagrams from now on (`true`) or on the streams.
    pub fn set_datagrams(&mut self, datagrams: bool) {
        self.datagrams = datagrams;
    }

    pub fn datagrams(&self) -> bool {
        self.datagrams
    }

    /// Forgets every picture in flight (the carrier changed): late
    /// acknowledgements of them are ignored; no key frame is asked for.
    pub fn forget_in_flight(&mut self) {
        self.received.clear();
        self.in_flight.clear();
        self.unacked_bytes = 0;
    }

    /// Drops the datagram pictures unacknowledged beyond the stale time.
    pub(super) fn forget_stale_datagrams(&mut self, now_us: u64, target_bps: u64) {
        let (stale_us, _) = self.stale_limits(target_bps);
        while let Some(oldest) = self.in_flight.front() {
            if now_us.saturating_sub(oldest.sent_us) <= stale_us {
                break;
            }
            self.unacked_bytes = self.unacked_bytes.saturating_sub(oldest.bytes);
            self.in_flight.pop_front();
        }
    }
}
