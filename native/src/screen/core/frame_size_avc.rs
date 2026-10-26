//! AVC level geometry and bounded in-band SPS validation before codec allocation.
use std::io;

use super::super::wire::invalid;
use super::FrameSize;

/// H.264 Annex A limits; values also exposed by Android's AVC capability model.
#[derive(Clone, Copy, Debug)]
pub(crate) struct AvcLimits {
    pub level: u32,
    pub macroblocks: u64,
    pub macroblocks_per_second: u64,
}
pub(crate) fn avc_limits(level: u32) -> Option<AvcLimits> {
    let (macroblocks, macroblocks_per_second) = match level {
        10 | 9 => (99, 1485),
        11 => (396, 3000),
        12 => (396, 6000),
        13 | 20 => (396, 11880),
        21 => (792, 19800),
        22 => (1620, 20250),
        30 => (1620, 40500),
        31 => (3600, 108000),
        32 => (5120, 216000),
        40 | 41 => (8192, 245760),
        42 => (8704, 522240),
        50 => (22080, 589824),
        51 => (36864, 983040),
        52 => (36864, 2073600),
        60 => (139264, 4177920),
        61 => (139264, 8355840),
        62 => (139264, 16711680),
        _ => return None,
    };
    Some(AvcLimits {
        level,
        macroblocks,
        macroblocks_per_second,
    })
}
pub(crate) fn macroblocks(size: FrameSize) -> u64 {
    u64::from(size.width.div_ceil(16)) * u64::from(size.height.div_ceil(16))
}
/// Largest even image inside `size` whose coded 16×16 blocks fit the budget.
/// This handles codec padding; cadence limits never call this function.
pub(crate) fn fit_macroblocks(size: FrameSize, max_blocks: u64) -> FrameSize {
    if size.width < 2 || size.height < 2 {
        return size;
    }
    if max_blocks == 0 {
        return FrameSize::new(0, 0);
    }
    let size = size.even();
    if macroblocks(size) <= max_blocks {
        return size;
    }
    let denominator = size.width.max(size.height);
    let candidate = |n: u32| {
        FrameSize::new(
            ((u64::from(size.width) * u64::from(n) / u64::from(denominator)) as u32 & !1).max(2),
            ((u64::from(size.height) * u64::from(n) / u64::from(denominator)) as u32 & !1).max(2),
        )
    };
    let (mut low, mut high) = (0, denominator);
    while low < high {
        let mid = low + (high - low).div_ceil(2);
        if macroblocks(candidate(mid)) <= max_blocks {
            low = mid;
        } else {
            high = mid - 1;
        }
    }
    candidate(low)
}
impl AvcLimits {
    pub(crate) fn supports(self, size: FrameSize, fps: u32) -> bool {
        let blocks = macroblocks(size);
        let side = (self.macroblocks.saturating_mul(8) as f64).sqrt().floor() as u64;
        size.width > 0
            && size.height > 0
            && blocks <= self.macroblocks
            && u64::from(size.width.div_ceil(16)) <= side
            && u64::from(size.height.div_ceil(16)) <= side
            && blocks
                .checked_mul(u64::from(fps))
                .is_some_and(|n| n <= self.macroblocks_per_second)
    }
}

/// Check every in-band SPS before a native decoder can allocate its coded picture.
/// The screen contract is progressive, 8-bit 4:2:0; parameter changes require a GOP.
pub(crate) fn check_avc_dimensions(
    data: &[u8],
    size: FrameSize,
    limits: super::super::frame_budget::FrameLimits,
) -> io::Result<()> {
    limits.check(size)?;
    let start = |at: usize| data.get(at..at + 3) == Some(&[0, 0, 1]);
    let mut at = 0;
    while at + 3 < data.len() {
        if !start(at) {
            at += 1;
            continue;
        }
        let from = at + 3;
        at = from + 1;
        while at + 3 < data.len() && !start(at) {
            at += 1;
        }
        let end = if at + 3 < data.len() { at } else { data.len() };
        if data[from] & 31 == 15 {
            return Err(invalid("AVC subset SPS is outside the screen contract"));
        }
        if data[from] & 31 != 7 {
            continue;
        }
        if end - from > 65536 {
            return Err(invalid("AVC SPS exceeds bounded parameter budget"));
        }
        let mut rbsp = Vec::with_capacity(end - from);
        let mut zeros = 0;
        for (offset, &byte) in data[from + 1..end].iter().enumerate() {
            if zeros >= 2 && byte == 3 {
                if data
                    .get(from + 2 + offset)
                    .filter(|_| from + 2 + offset < end)
                    .is_none_or(|n| *n > 3)
                {
                    return Err(invalid("invalid AVC emulation prevention"));
                }
                zeros = 0;
                continue;
            }
            rbsp.push(byte);
            zeros = if byte == 0 { zeros + 1 } else { 0 };
        }
        let mut bits = AvcBits { data: &rbsp, at: 0 };
        let profile = bits.read(8)?;
        let constraints = bits.read(8)?;
        let level = bits.read(8)?;
        if bits.ue()? > 31 {
            return Err(invalid("AVC SPS id exceeds Annex A"));
        }
        if profile == 100 {
            if bits.ue()? != 1 || bits.ue()? != 0 || bits.ue()? != 0 {
                return Err(invalid("screen AVC requires 8-bit 4:2:0"));
            }
            bits.read(1)?;
            if bits.read(1)? != 0 {
                for i in 0..8 {
                    if bits.read(1)? != 0 {
                        let mut last = 8i64;
                        let mut next = 8i64;
                        for _ in 0..if i < 6 { 16 } else { 64 } {
                            if next != 0 {
                                next = (last + bits.se()? + 256).rem_euclid(256);
                            }
                            if next != 0 {
                                last = next;
                            }
                        }
                    }
                }
            }
        } else if profile != 66 && profile != 77 {
            return Err(invalid("unsupported screen AVC SPS profile"));
        }
        if bits.ue()? > 12 {
            return Err(invalid("AVC frame number exceeds Annex A"));
        }
        match bits.ue()? {
            0 => {
                if bits.ue()? > 12 {
                    return Err(invalid("AVC POC length exceeds Annex A"));
                }
            }
            1 => {
                bits.read(1)?;
                bits.se()?;
                bits.se()?;
                let count = bits.ue()?;
                if count > 256 {
                    return Err(invalid("AVC POC cycle exceeds budget"));
                }
                for _ in 0..count {
                    bits.se()?;
                }
            }
            2 => {}
            _ => return Err(invalid("invalid AVC picture order type")),
        }
        let refs = bits.ue()?;
        if refs > 16 {
            return Err(invalid("AVC reference count exceeds Annex A"));
        }
        bits.read(1)?;
        let width = bits
            .ue()?
            .checked_add(1)
            .and_then(|n| n.checked_mul(16))
            .ok_or_else(|| invalid("AVC width overflow"))?;
        let height = bits
            .ue()?
            .checked_add(1)
            .and_then(|n| n.checked_mul(16))
            .ok_or_else(|| invalid("AVC height overflow"))?;
        if bits.read(1)? != 1 {
            return Err(invalid("interlaced AVC is outside the screen contract"));
        }
        bits.read(1)?;
        let mut crop = [0u32; 4];
        if bits.read(1)? != 0 {
            for value in &mut crop {
                *value = bits.ue()?;
            }
        }
        let visible_w = width.checked_sub(
            crop[0]
                .checked_add(crop[1])
                .and_then(|n| n.checked_mul(2))
                .ok_or_else(|| invalid("AVC crop overflow"))?,
        );
        let visible_h = height.checked_sub(
            crop[2]
                .checked_add(crop[3])
                .and_then(|n| n.checked_mul(2))
                .ok_or_else(|| invalid("AVC crop overflow"))?,
        );
        let coded = FrameSize::new(width, height);
        let level = if level == 11 && constraints & 0x10 != 0 && (profile == 66 || profile == 77) {
            9
        } else {
            level
        };
        let level = avc_limits(level).ok_or_else(|| invalid("unknown AVC level"))?;
        if visible_w != Some(size.width)
            || visible_h != Some(size.height)
            || !level.supports(coded, 0)
        {
            return Err(invalid(
                "AVC SPS dimensions or level contradict the frame contract",
            ));
        }
        // Coded padding also consumes decoder memory, even when it is cropped away.
        limits.check(coded)?;
    }
    Ok(())
}
struct AvcBits<'a> {
    data: &'a [u8],
    at: usize,
}
impl AvcBits<'_> {
    fn read(&mut self, count: usize) -> io::Result<u32> {
        if count > 32
            || self
                .at
                .checked_add(count)
                .is_none_or(|n| n > self.data.len() * 8)
        {
            return Err(invalid("truncated AVC SPS"));
        }
        let mut value = 0;
        for _ in 0..count {
            value = (value << 1) | u32::from((self.data[self.at / 8] >> (7 - self.at % 8)) & 1);
            self.at += 1;
        }
        Ok(value)
    }
    fn ue(&mut self) -> io::Result<u32> {
        let mut zeros = 0;
        while self.read(1)? == 0 {
            zeros += 1;
            if zeros > 30 {
                return Err(invalid("AVC Exp-Golomb overflow"));
            }
        }
        Ok((1u32 << zeros) - 1 + self.read(zeros)?)
    }
    fn se(&mut self) -> io::Result<i64> {
        let n = i64::from(self.ue()?);
        Ok(if n & 1 == 0 { -n / 2 } else { (n + 1) / 2 })
    }
}
