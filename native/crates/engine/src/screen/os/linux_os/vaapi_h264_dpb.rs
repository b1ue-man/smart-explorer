//! Picture order count and reference pictures of the VA-API H.264 decoder
//! (BK4, B-38): H.264 (08/2021) 8.2.1 picture order count (types 0 and 2),
//! 8.2.4 reference list 0 of P slices with its modification, 8.2.5
//! reference marking (sliding window and memory management operations).
//! Frames only; a gap in `frame_num` is refused.
use std::io;

use super::vaapi_h264_slice::{ListModification, Mmco, SliceHeader};
use super::vaapi_h264_syntax::Sps;
use crate::screen::core::wire::invalid;

/// One picture in the decoded picture buffer.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct RefPicture {
    pub surface: u32,
    pub frame_num: u32,
    /// `LongTermFrameIdx` of a long-term reference.
    pub long_term: Option<u32>,
    pub top_poc: i32,
    pub bottom_poc: i32,
}

/// Reference state across the pictures of one picture group.
#[derive(Clone, Debug, Default)]
pub(super) struct Dpb {
    pub refs: Vec<RefPicture>,
    max_long_term_idx: Option<u32>,
    prev_poc_msb: i32,
    prev_poc_lsb: i32,
    prev_frame_num_offset: i64,
    prev_frame_num: u32,
    prev_ref_frame_num: Option<u32>,
}

/// `FrameNumWrap` (8-27) of a short-term reference, also its `PicNum`.
fn wrap(picture: &RefPicture, current: u32, max: u32) -> i64 {
    if picture.frame_num > current {
        i64::from(picture.frame_num) - i64::from(max)
    } else {
        i64::from(picture.frame_num)
    }
}

impl Dpb {
    /// The picture order counts (top, bottom) of `slice` (8.2.1).
    pub(super) fn poc(&mut self, slice: &SliceHeader, sps: &Sps) -> io::Result<(i32, i32)> {
        if slice.idr {
            self.prev_poc_msb = 0;
            self.prev_poc_lsb = 0;
            self.prev_frame_num_offset = 0;
            self.prev_frame_num = 0;
        }
        if sps.poc_type == 0 {
            let max = 1i32 << sps.log2_max_poc_lsb;
            let lsb = slice.poc_lsb as i32;
            let previous = self.prev_poc_lsb;
            let msb = if lsb < previous && previous - lsb >= max / 2 {
                self.prev_poc_msb.checked_add(max)
            } else if lsb > previous && lsb - previous > max / 2 {
                self.prev_poc_msb.checked_sub(max)
            } else {
                Some(self.prev_poc_msb)
            }
            .ok_or_else(|| invalid("H.264 POC overflow"))?;
            let top = msb
                .checked_add(lsb)
                .ok_or_else(|| invalid("H.264 POC overflow"))?;
            if slice.nal_ref_idc != 0 {
                self.prev_poc_msb = msb;
                self.prev_poc_lsb = lsb;
            }
            return Ok((top, top.saturating_add(slice.delta_poc_bottom)));
        }
        let max = i64::from(sps.max_frame_num());
        let offset = if slice.idr {
            0
        } else if self.prev_frame_num > slice.frame_num {
            self.prev_frame_num_offset
                .checked_add(max)
                .ok_or_else(|| invalid("H.264 POC overflow"))?
        } else {
            self.prev_frame_num_offset
        };
        let base = offset
            .checked_add(i64::from(slice.frame_num))
            .and_then(|sum| sum.checked_mul(2))
            .ok_or_else(|| invalid("H.264 POC overflow"))?;
        let poc = match (slice.idr, slice.nal_ref_idc) {
            (true, _) => 0,
            (false, 0) => base - 1,
            (false, _) => base,
        };
        self.prev_frame_num_offset = offset;
        let poc = i32::try_from(poc).map_err(|_| invalid("H.264 POC overflow"))?;
        Ok((poc, poc))
    }

    /// Checks `frame_num` against the last reference (no gaps, 8.2.5.2).
    pub(super) fn check_gap(&self, slice: &SliceHeader, sps: &Sps) -> io::Result<()> {
        if slice.idr {
            return Ok(());
        }
        let Some(previous) = self.prev_ref_frame_num else {
            return Err(invalid("H.264 picture without a preceding IDR"));
        };
        let next = (previous + 1) % sps.max_frame_num();
        if slice.frame_num != previous && slice.frame_num != next {
            return Err(io::Error::new(
                io::ErrorKind::Unsupported,
                "H.264 gap in frame_num",
            ));
        }
        Ok(())
    }

    /// Reference list 0 of a P slice (8.2.4.2.1, 8.2.4.3). Entries without
    /// a picture stay "no reference picture" (legal while no macroblock
    /// refers to them; VA-API gets invalid entries for them).
    pub(super) fn list0(&self, slice: &SliceHeader, sps: &Sps) -> io::Result<Vec<RefPicture>> {
        let max = sps.max_frame_num();
        let current = slice.frame_num;
        let mut shorts: Vec<&RefPicture> =
            self.refs.iter().filter(|r| r.long_term.is_none()).collect();
        shorts.sort_by_key(|r| std::cmp::Reverse(wrap(r, current, max)));
        let mut longs: Vec<&RefPicture> =
            self.refs.iter().filter(|r| r.long_term.is_some()).collect();
        longs.sort_by_key(|r| r.long_term);
        let active = slice.num_ref_idx_l0_minus1 as usize + 1;
        let mut list: Vec<Option<RefPicture>> =
            shorts.into_iter().chain(longs).map(|r| Some(*r)).collect();
        list.resize(active.max(list.len()), None);
        list.truncate(active);
        let mut pred = i64::from(current);
        for (index, modification) in slice.modifications.iter().enumerate() {
            if index >= active {
                return Err(invalid("H.264 reference list modification beyond the list"));
            }
            let picked = match *modification {
                ListModification::Subtract(diff) => {
                    self.short_term(&mut pred, -i64::from(diff), current, max)
                }
                ListModification::Add(diff) => {
                    self.short_term(&mut pred, i64::from(diff), current, max)
                }
                ListModification::LongTerm(idx) => {
                    self.refs.iter().find(|r| r.long_term == Some(idx))
                }
            };
            let picked =
                *picked.ok_or_else(|| invalid("H.264 reference list names a missing picture"))?;
            list.insert(index, Some(picked));
            let mut kept = list[..=index].to_vec();
            let rest = list[index + 1..].iter().filter(|e| **e != Some(picked));
            kept.extend(rest.copied());
            list = kept;
            list.truncate(active);
        }
        // Missing entries can only trail the list (modifications insert
        // existing pictures in front), so dropping them keeps every index.
        Ok(list.into_iter().flatten().collect())
    }

    /// The short-term picture `pred + step` names (8.2.4.3.1).
    fn short_term(&self, pred: &mut i64, step: i64, current: u32, max: u32) -> Option<&RefPicture> {
        if step == 0 || step.unsigned_abs() > u64::from(max) {
            return None;
        }
        let max = i64::from(max);
        let mut no_wrap = *pred + step;
        if no_wrap < 0 {
            no_wrap += max;
        } else if no_wrap >= max {
            no_wrap -= max;
        }
        *pred = no_wrap;
        let pic_num = if no_wrap > i64::from(current) {
            no_wrap - max
        } else {
            no_wrap
        };
        self.refs
            .iter()
            .find(|r| r.long_term.is_none() && wrap(r, current, max as u32) == pic_num)
    }

    /// Marks the decoded picture (8.2.5) on `surface`; the surface of every
    /// picture that leaves the buffer is free again.
    pub(super) fn mark(
        &mut self,
        slice: &SliceHeader,
        sps: &Sps,
        surface: u32,
        poc: (i32, i32),
    ) -> io::Result<()> {
        if slice.nal_ref_idc == 0 {
            self.prev_frame_num = slice.frame_num;
            return Ok(());
        }
        let mut current = RefPicture {
            surface,
            frame_num: slice.frame_num,
            long_term: None,
            top_poc: poc.0,
            bottom_poc: poc.1,
        };
        let mut next_frame_num = slice.frame_num;
        if slice.idr {
            self.refs.clear();
            self.max_long_term_idx = slice.long_term_reference.then_some(0);
            current.long_term = self.max_long_term_idx;
        } else if let Some(operations) = &slice.mmco {
            for operation in operations {
                if self.apply(*operation, slice, sps, &mut current)? {
                    next_frame_num = 0;
                }
            }
        } else {
            self.slide(slice, sps);
        }
        if self.refs.len() >= sps.max_num_ref_frames.max(1) as usize {
            return Err(invalid("H.264 decoded picture buffer overflow"));
        }
        self.refs.push(current);
        self.prev_frame_num = next_frame_num;
        self.prev_ref_frame_num = Some(next_frame_num);
        Ok(())
    }

    /// The sliding window (8.2.5.3): a full buffer drops its oldest
    /// short-term reference.
    fn slide(&mut self, slice: &SliceHeader, sps: &Sps) {
        let max = sps.max_frame_num();
        if self.refs.len() < sps.max_num_ref_frames.max(1) as usize {
            return;
        }
        let oldest = self
            .refs
            .iter()
            .enumerate()
            .filter(|(_, r)| r.long_term.is_none())
            .min_by_key(|(_, r)| wrap(r, slice.frame_num, max))
            .map(|(index, _)| index);
        if let Some(index) = oldest {
            self.refs.remove(index);
        }
    }

    /// One memory management operation (8.2.5.4); `true` for operation 5,
    /// after which the picture counts as `frame_num` 0 with POC 0.
    fn apply(
        &mut self,
        operation: Mmco,
        slice: &SliceHeader,
        sps: &Sps,
        current: &mut RefPicture,
    ) -> io::Result<bool> {
        let (frame_num, max) = (slice.frame_num, sps.max_frame_num());
        let pic_num = i64::from(frame_num);
        match operation {
            Mmco::UnmarkShort(diff) => {
                let target = pic_num - i64::from(diff);
                let keep =
                    |r: &RefPicture| r.long_term.is_some() || wrap(r, frame_num, max) != target;
                self.refs.retain(keep);
            }
            Mmco::UnmarkLong(idx) => self.refs.retain(|r| r.long_term != Some(idx)),
            Mmco::ShortToLong(diff, idx) => {
                if self.max_long_term_idx.is_none_or(|max_idx| idx > max_idx) {
                    return Err(invalid(
                        "H.264 LongTermFrameIdx exceeds MaxLongTermFrameIdx",
                    ));
                }
                let target = pic_num - i64::from(diff);
                self.refs.retain(|r| r.long_term != Some(idx));
                let found = self
                    .refs
                    .iter()
                    .position(|r| r.long_term.is_none() && wrap(r, frame_num, max) == target)
                    .ok_or_else(|| invalid("H.264 marking names a missing picture"))?;
                self.refs[found].long_term = Some(idx);
            }
            Mmco::MaxLongTermIdx(plus1) => {
                let limit = plus1.checked_sub(1);
                self.max_long_term_idx = limit;
                let keep = |r: &RefPicture| {
                    r.long_term
                        .is_none_or(|idx| limit.is_some_and(|max| idx <= max))
                };
                self.refs.retain(keep);
            }
            Mmco::UnmarkAll => {
                self.refs.clear();
                self.max_long_term_idx = None;
                current.frame_num = 0;
                let base = current.top_poc.min(current.bottom_poc);
                current.top_poc = current.top_poc.saturating_sub(base);
                current.bottom_poc = current.bottom_poc.saturating_sub(base);
                self.prev_poc_msb = 0;
                self.prev_poc_lsb = current.top_poc;
                self.prev_frame_num_offset = 0;
                return Ok(true);
            }
            Mmco::CurrentToLong(idx) => {
                if self.max_long_term_idx.is_none_or(|max_idx| idx > max_idx) {
                    return Err(invalid(
                        "H.264 LongTermFrameIdx exceeds MaxLongTermFrameIdx",
                    ));
                }
                self.refs.retain(|r| r.long_term != Some(idx));
                current.long_term = Some(idx);
            }
        }
        Ok(false)
    }
}
