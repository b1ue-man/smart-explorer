//! Picture order count, reference picture set and reference lists of the
//! VA-API HEVC decoder (plan 2026-11-14 „Bildkette“ BK4b, B-38): ITU-T
//! H.265 (08/2021) 8.3.1 (POC with `prevTid0Pic`), 8.3.2 (RPS: pictures
//! outside it leave the buffer), 8.3.4 (`RefPicList0/1` with their
//! modification). The screen shows every picture when it is decoded, so a
//! picture that references a later one (`PocStCurrAfter`, the mark of
//! reordering) and leading pictures (RADL/RASL) are refused with
//! `ErrorKind::Unsupported`; low-delay B slices (both lists on earlier
//! pictures, as Intel's VA-API and Quick Sync write P pictures) decode.
use std::io;

use super::vaapi_ffi::VaId;
use super::vaapi_hevc_abi::{
    VaPictureHevc, HEVC_REF_INVALID, VA_PICTURE_HEVC_RPS_ST_CURR_AFTER,
    VA_PICTURE_HEVC_RPS_ST_CURR_BEFORE,
};
use super::vaapi_hevc_slice::{SliceHeader, NAL_CRA, SLICE_B, SLICE_I};
use super::vaapi_hevc_syntax::{unsupported, Sps};
use crate::screen::core::wire::invalid;

/// One decoded picture kept for reference.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct DpbPicture {
    pub surface: VaId,
    pub poc: i32,
}

/// The references of the picture being decoded.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(super) struct PictureRefs {
    pub poc: i32,
    /// `ReferenceFrames[]` of VA-API: every picture of the RPS with its
    /// `VA_PICTURE_HEVC_RPS_*` flag.
    pub frames: Vec<VaPictureHevc>,
    pub before: Vec<i32>,
    pub after: Vec<i32>,
}

/// The decoded picture buffer of one picture group.
#[derive(Clone, Debug, Default)]
pub(super) struct Dpb {
    pub pictures: Vec<DpbPicture>,
    prev_tid0_poc: Option<i32>,
}

/// Whether a NAL type may be decoded here: trailing pictures, temporal
/// and step-wise sub-layer access, IDR and CRA.
pub(super) fn check_nal_type(nal_type: u8) -> io::Result<()> {
    match nal_type {
        0..=5 | 19..=NAL_CRA => Ok(()),
        6..=9 => Err(unsupported("HEVC leading pictures reorder pictures")),
        16..=18 => Err(unsupported("HEVC broken links are not decoded here")),
        _ => Err(invalid("HEVC reserved picture type")),
    }
}

impl Dpb {
    /// The picture order count of the picture of `h` (8.3.1); an IRAP
    /// starts anew (IDR always, a CRA as the first picture of a group).
    pub(super) fn poc(&mut self, h: &SliceHeader, sps: &Sps, temporal_id: u8) -> io::Result<i32> {
        let max = 1i64 << sps.log2_max_poc_lsb;
        let lsb = i64::from(h.poc_lsb);
        let restart = h.idr() || (h.nal_type == NAL_CRA && self.prev_tid0_poc.is_none());
        let msb = if restart {
            0
        } else {
            let previous = i64::from(
                self.prev_tid0_poc
                    .ok_or_else(|| invalid("HEVC picture group does not start with an IRAP"))?,
            );
            let prev_lsb = previous & (max - 1);
            let prev_msb = previous - prev_lsb;
            if lsb < prev_lsb && prev_lsb - lsb >= max / 2 {
                prev_msb + max
            } else if lsb > prev_lsb && lsb - prev_lsb > max / 2 {
                prev_msb - max
            } else {
                prev_msb
            }
        };
        let poc = i32::try_from(msb + if h.idr() { 0 } else { lsb })
            .map_err(|_| invalid("HEVC POC overflow"))?;
        let sub_layer_non_reference = h.nal_type < 16 && h.nal_type % 2 == 0;
        if temporal_id == 0 && !sub_layer_non_reference {
            self.prev_tid0_poc = Some(poc);
        }
        Ok(poc)
    }

    /// Applies the RPS of `h` for the picture `poc` (8.3.2): pictures
    /// outside it leave the buffer; the current references must be there.
    pub(super) fn apply_rps(&mut self, h: &SliceHeader, poc: i32) -> io::Result<PictureRefs> {
        if h.idr() {
            self.pictures.clear();
        }
        if h.irap() && h.slice_type != SLICE_I {
            return Err(invalid("HEVC IRAP picture with an inter slice"));
        }
        let mut refs = PictureRefs {
            poc,
            ..PictureRefs::default()
        };
        let mut kept: Vec<(DpbPicture, u32)> = Vec::new();
        let rps = &h.rps;
        let negatives = rps.negative.iter().map(|entry| (*entry, true));
        let positives = rps.positive.iter().map(|entry| (*entry, false));
        for ((delta, used), negative) in negatives.chain(positives) {
            let target = poc
                .checked_add(delta)
                .ok_or_else(|| invalid("HEVC POC overflow"))?;
            let found = self.pictures.iter().find(|p| p.poc == target).copied();
            let flag = if !used {
                0
            } else if negative {
                VA_PICTURE_HEVC_RPS_ST_CURR_BEFORE
            } else {
                VA_PICTURE_HEVC_RPS_ST_CURR_AFTER
            };
            match found {
                Some(picture) => kept.push((picture, flag)),
                None if used => return Err(invalid("HEVC reference picture is missing")),
                None => {}
            }
            if used && negative {
                refs.before.push(target);
            } else if used {
                refs.after.push(target);
            }
        }
        if !refs.after.is_empty() {
            return Err(unsupported(
                "HEVC references to later pictures reorder pictures",
            ));
        }
        self.pictures = kept.iter().map(|(picture, _)| *picture).collect();
        refs.frames = kept
            .iter()
            .map(|(picture, flag)| VaPictureHevc {
                picture_id: picture.surface,
                pic_order_cnt: picture.poc,
                flags: *flag,
                ..VaPictureHevc::default()
            })
            .collect();
        if refs.frames.len() > 15 {
            return Err(invalid("HEVC RPS larger than the DPB"));
        }
        Ok(refs)
    }

    /// Keeps the decoded picture for reference.
    pub(super) fn insert(&mut self, surface: VaId, poc: i32, sps: &Sps) -> io::Result<()> {
        if self.pictures.len() >= sps.max_dec_pic_buffering.max(1) as usize + 1 {
            return Err(invalid("HEVC decoded picture buffer overflow"));
        }
        self.pictures.push(DpbPicture { surface, poc });
        Ok(())
    }

    /// Whether `surface` holds a reference.
    pub(super) fn holds(&self, surface: VaId) -> bool {
        self.pictures.iter().any(|p| p.surface == surface)
    }
}

/// `RefPicList0` and `RefPicList1` of a slice (8.3.4) as indices into
/// `refs.frames` (`HEVC_REF_INVALID` beyond the active entries).
pub(super) fn ref_lists(h: &SliceHeader, refs: &PictureRefs) -> io::Result<[[u8; 15]; 2]> {
    let mut out = [[HEVC_REF_INVALID; 15]; 2];
    if h.slice_type == SLICE_I {
        return Ok(out);
    }
    let index_of = |poc: i32| {
        refs.frames
            .iter()
            .position(|frame| frame.pic_order_cnt == poc)
            .map(|at| at as u8)
            .ok_or_else(|| invalid("HEVC reference list names a missing picture"))
    };
    let total = h.total_curr() as usize;
    let lists = if h.slice_type == SLICE_B { 2 } else { 1 };
    for list in 0..lists {
        let active = h.num_ref_idx[list] as usize;
        let (first, second) = if list == 0 {
            (&refs.before, &refs.after)
        } else {
            (&refs.after, &refs.before)
        };
        let order: Vec<i32> = first.iter().chain(second.iter()).copied().collect();
        if order.len() != total || total == 0 {
            return Err(invalid("HEVC reference set and slice disagree"));
        }
        let temp: Vec<i32> = order
            .iter()
            .cycle()
            .take(active.max(total))
            .copied()
            .collect();
        for index in 0..active {
            let entry = match &h.list_entry[list] {
                Some(entries) => *entries
                    .get(index)
                    .ok_or_else(|| invalid("HEVC list modification too short"))?
                    as usize,
                None => index,
            };
            let poc = *temp
                .get(entry)
                .ok_or_else(|| invalid("HEVC list modification out of range"))?;
            out[list][index] = index_of(poc)?;
        }
    }
    Ok(out)
}
