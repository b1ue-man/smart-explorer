//! Portable image geometry and derived resource/packet bounds.
use std::io;
use serde::{Deserialize,Serialize};
use super::wire::invalid;

/// Preserved v1 side bound; versioned capabilities may establish a larger limit.
pub(crate) const MAX_FRAME_SIDE: u32 = 4096;
/// Preserved v1 pixel budget: 4096 × 2304 = 36 864 AVC macroblocks.
pub(crate) const MAX_FRAME_PIXELS: u64 = 4096 * 2304;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub(crate) struct FrameSize {
    pub width: u32,
    pub height: u32,
}

impl FrameSize {
    /// Largest output in landscape orientation (C2 „Grenzen mit Grund“).
    pub(crate) const MAX: Self = Self::new(4096, 2304);

    pub(crate) const fn new(width: u32, height: u32) -> Self {
        Self { width, height }
    }

    pub(crate) fn pixels(self) -> u64 {
        u64::from(self.width) * u64::from(self.height)
    }

    /// A frame that may be allocated for data from the network.
    pub(crate) fn is_valid_frame(self) -> bool {
        (1..=MAX_FRAME_SIDE).contains(&self.width)
            && (1..=MAX_FRAME_SIDE).contains(&self.height)
            && self.pixels() <= MAX_FRAME_PIXELS
    }

    pub(crate) fn check_frame(self) -> io::Result<()> {
        if self.is_valid_frame() {
            Ok(())
        } else {
            Err(invalid(
                "screen frame size is outside 1..=4096 or above 4096 x 2304 pixels",
            ))
        }
    }

    /// Bytes of a tight BGRA frame; only meaningful for valid frames.
    pub(crate) fn bgra_len(self) -> usize {
        self.checked_bgra_len().unwrap_or(0)
    }

    pub(crate) fn checked_bgra_len(self) -> io::Result<usize> {
        usize::try_from(self.width).ok().and_then(|w| usize::try_from(self.height).ok()
            .and_then(|h| w.checked_mul(h))).and_then(|n| n.checked_mul(4))
            .ok_or_else(|| invalid("BGRA byte size overflow"))
    }

    /// The largest size with the same aspect ratio within `max`; never larger
    /// than `self` and never below 1 × 1.
    pub(crate) fn fit_within(self, max: Self) -> Self {
        if self.width <= max.width && self.height <= max.height {
            return self;
        }
        if self.width == 0 || self.height == 0 || max.width == 0 || max.height == 0 {
            return Self::new(self.width.min(max.width), self.height.min(max.height));
        }
        let (width, height) = (u64::from(self.width), u64::from(self.height));
        let (max_width, max_height) = (u64::from(max.width), u64::from(max.height));
        if width * max_height >= height * max_width {
            Self::new(max.width.max(1), (height * max_width / width).max(1) as u32)
        } else {
            Self::new(
                (width * max_height / height).max(1) as u32,
                max.height.max(1),
            )
        }
    }

    /// `self` × `numerator` / `denominator`, at least 1 × 1.
    pub(crate) fn scale(self, numerator: u32, denominator: u32) -> Self {
        let scale = |side: u32| {
            let scaled = u64::from(side) * u64::from(numerator) / u64::from(denominator.max(1));
            scaled.clamp(1, u64::from(u32::MAX)) as u32
        };
        Self::new(scale(self.width), scale(self.height))
    }

    /// Rounded down to even sides (4:2:0 encoders), at least 2 × 2.
    pub(crate) fn even(self) -> Self {
        Self::new((self.width & !1).max(2), (self.height & !1).max(2))
    }

    /// Size of a 4:2:0 chroma plane.
    pub(crate) fn chroma(self) -> Self {
        Self::new(self.width.div_ceil(2), self.height.div_ceil(2))
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub(crate) struct Rect {
    pub x: u32,
    pub y: u32,
    pub width: u32,
    pub height: u32,
}

impl Rect {
    pub(crate) fn fits(&self, size: FrameSize) -> bool {
        self.width > 0
            && self.height > 0
            && u64::from(self.x) + u64::from(self.width) <= u64::from(size.width)
            && u64::from(self.y) + u64::from(self.height) <= u64::from(size.height)
    }
}


/// Payload bounds for a single admitted frame, independent of sender queue length.
#[derive(Clone,Copy,Debug)]
pub(crate) struct PacketBudget {
    pub raw_bytes:usize,pub records:usize,pub encoded_bytes:usize,
    pub record_bytes:usize,pub pull_bytes:usize,
}
/// Relay GOP + record + maximum contract extension, plus JNI head/identity.
pub(crate) const RECORD_BUFFER_OVERHEAD:usize=
    super::wire_binary::GOP_HEADER_BYTES+super::wire_binary::RECORD_HEADER_BYTES+1026+32;
impl PacketBudget {
    pub(crate) fn for_limits(limits:super::frame_budget::FrameLimits)->io::Result<Self> {
        // Bare limits describe source/private legacy buffers, not a v2 grant.
        Self::for_limits_mode(limits,true)
    }
    pub(crate) fn for_contract(contract:Option<super::frame_budget::FrameContract>)->io::Result<Self> {
        Self::for_limits_mode(contract.map_or(super::frame_budget::FrameLimits::LEGACY,|c|c.limits),contract.is_none())
    }
    fn for_limits_mode(limits:super::frame_budget::FrameLimits,legacy:bool)->io::Result<Self> {
        limits.validate()?;
        let images=if legacy {super::frame_budget::WORKING_IMAGES} else {super::frame_budget::VERSIONED_WORKING_IMAGES};
        let pixels=limits.max_pixels.min(limits.working_bytes/images/4).min(limits.max_size.pixels());
        let columns=u64::from(limits.max_size.width.div_ceil(64));let rows=u64::from(limits.max_size.height.div_ceil(64));
        let tiles=(pixels.div_ceil(4096)+columns+rows+1).min(columns*rows).min(pixels);
        let to_size=|n:u64|usize::try_from(n).map_err(|_|invalid("frame packet budget exceeds address space"));
        let raw_bytes=to_size(pixels.checked_mul(4).and_then(|n|n.checked_add(8)).ok_or_else(||invalid("raw frame packet overflow"))?)?;
        let records=to_size(tiles.checked_add(4).ok_or_else(||invalid("frame record count overflow"))?)?;
        let tile= lz4_flex::block::get_maximum_output_size(64*64*4) as u64;
        // One full Video record, two stamps, one move list with at most 2*tiles entries,
        // lossless LZ4 tiles, record lengths and the bounded encoder-name/header fields.
        let image=pixels.checked_mul(4).ok_or_else(||invalid("image budget overflow"))?;
        // Reserve the pixel/metadata ownership first. Thirty-two live record
        // copies plus the queue's video record share the remaining bytes.
        let tile_pixels=u64::from(limits.max_size.width.min(64))*u64::from(limits.max_size.height.min(64));
        let tile_raw=to_size(tile_pixels.min(pixels)*4)?;
        let floor=lz4_flex::block::get_maximum_output_size(tile_raw).checked_add(41)
            .ok_or_else(||invalid("tile budget overflow"))?;
        let overhead=image/255+tiles*512+1024+(RECORD_BUFFER_OVERHEAD as u64)*32;
        let available=limits.working_bytes.saturating_sub(image*20).saturating_sub(overhead);
        let record_bytes=if legacy {super::wire_binary::MAX_RECORD_BYTES}
            else {to_size(available/33)?.min(to_size(image)?).max(floor).min(super::wire_binary::MAX_RECORD_BYTES)};
        let encoded=if legacy { tiles.checked_mul(tile+41+super::wire_binary::RECORD_HEADER_BYTES as u64+48).and_then(|n|n.checked_add(super::wire_binary::MAX_RECORD_BYTES as u64))
            .and_then(|n|n.checked_add(2*25+29+4*super::wire_binary::RECORD_HEADER_BYTES as u64+24+256+64)).ok_or_else(||invalid("encoded frame packet overflow"))?
            } else {
                // LZ4 worst case over the actual pixels, per-tile protocol,
                // move-list and JNI identity overhead, one bounded video AU.
                image.checked_add(image/255).and_then(|n|n.checked_add(tiles*512))
                    .and_then(|n|n.checked_add(record_bytes as u64+1024))
                    .ok_or_else(||invalid("encoded frame packet overflow"))?
            };
        if !legacy {
            // Thirteen image slots cover native/decoder images, the sole pull
            // buffer, Full/Preview snapshots and active/next scaling sources.
            // Seven further slots reserve resampling/output ownership. Android
            // leases actual source/target arrays, fixed filter rows and retained
            // outputs against this remainder before allocating. Codec/driver
            // storage is outside this product-buffer contract.
            let peak=image.checked_mul(20).and_then(|n|n.checked_add((record_bytes as u64+RECORD_BUFFER_OVERHEAD as u64)*32))
                .and_then(|n|n.checked_add(encoded.saturating_sub(image)));
            if peak.is_none_or(|n|n>limits.working_bytes) {return Err(invalid("screen product buffers exceed negotiated working memory"));}
        }
        let pull_bytes=to_size(image.max(record_bytes as u64).checked_add(32).ok_or_else(||invalid("pull size overflow"))?)?;
        Ok(Self {raw_bytes,records,encoded_bytes:to_size(encoded)?,record_bytes,pull_bytes})
    }
}

/// Producer-side rejection keeps the worker alive so the existing Tiles
/// fallback can reuse its source, input ownership and capture connection.
pub(crate) fn validate_encoded_budget(contract:Option<super::frame_budget::FrameContract>,
    codec:super::negotiate::CodecKind,records:&[Vec<u8>])->io::Result<()> {
    let Some(contract)=contract else {return Ok(());};
    contract.validate()?;
    let budget=PacketBudget::for_contract(Some(contract))?;
    let total=records.iter().try_fold(0usize,|n,r|n.checked_add(r.len()).and_then(|n|n.checked_add(super::wire_binary::RECORD_HEADER_BYTES)));
    if records.len()>budget.records || total.is_none_or(|n|n>budget.encoded_bytes)
        || ((contract.hybrid_regions || codec==super::negotiate::CodecKind::H264)
            && records.iter().any(|r|r.len()>budget.record_bytes)) {
        return Err(invalid("encoder exceeds negotiated frame-buffer budget"));
    }
    Ok(())
}

/// H.264 Annex A limits; values also exposed by Android's AVC capability model.
#[derive(Clone,Copy,Debug)]
pub(crate) struct AvcLimits {pub level:u32,pub macroblocks:u64,pub macroblocks_per_second:u64}
pub(crate) fn avc_limits(level:u32)->Option<AvcLimits> {
    let (macroblocks,macroblocks_per_second)=match level {
        10|9=>(99,1485),11=>(396,3000),12=>(396,6000),13|20=>(396,11880),21=>(792,19800),
        22=>(1620,20250),30=>(1620,40500),31=>(3600,108000),32=>(5120,216000),
        40|41=>(8192,245760),42=>(8704,522240),50=>(22080,589824),
        51=>(36864,983040),52=>(36864,2073600),60=>(139264,4177920),
        61=>(139264,8355840),62=>(139264,16711680),_=>return None,
    };Some(AvcLimits {level,macroblocks,macroblocks_per_second})
}
pub(crate) fn macroblocks(size:FrameSize)->u64 {u64::from(size.width.div_ceil(16))*u64::from(size.height.div_ceil(16))}
/// Largest even image inside `size` whose coded 16×16 blocks fit the budget.
/// This handles codec padding; cadence limits never call this function.
pub(crate) fn fit_macroblocks(size:FrameSize,max_blocks:u64)->FrameSize {
    if size.width<2 || size.height<2 {return size;}
    if max_blocks==0 {return FrameSize::new(0,0);}
    let size=size.even();
    if macroblocks(size)<=max_blocks {return size;}
    let denominator=size.width.max(size.height);
    let candidate=|n:u32|FrameSize::new(((u64::from(size.width)*u64::from(n)/u64::from(denominator)) as u32 & !1).max(2),
        ((u64::from(size.height)*u64::from(n)/u64::from(denominator)) as u32 & !1).max(2));
    let (mut low,mut high)=(0,denominator);
    while low<high {let mid=low+(high-low).div_ceil(2);if macroblocks(candidate(mid))<=max_blocks {low=mid;} else {high=mid-1;}}
    candidate(low)
}
impl AvcLimits {
    pub(crate) fn supports(self,size:FrameSize,fps:u32)->bool {
        let blocks=macroblocks(size);let side=(self.macroblocks.saturating_mul(8) as f64).sqrt().floor() as u64;
        size.width>0 && size.height>0 && blocks<=self.macroblocks && u64::from(size.width.div_ceil(16))<=side
            && u64::from(size.height.div_ceil(16))<=side && blocks.checked_mul(u64::from(fps)).is_some_and(|n|n<=self.macroblocks_per_second)
    }
}

/// Check every in-band SPS before a native decoder can allocate its coded picture.
/// The screen contract is progressive, 8-bit 4:2:0; parameter changes require a GOP.
pub(crate) fn check_avc_dimensions(data:&[u8],size:FrameSize,limits:super::frame_budget::FrameLimits)->io::Result<()> {
    limits.check(size)?;
    let start=|at:usize|data.get(at..at+3)==Some(&[0,0,1]);
    let mut at=0;
    while at+3<data.len() {
        if !start(at) {at+=1;continue;}
        let from=at+3;at=from+1;
        while at+3<data.len() && !start(at) {at+=1;}
        let end=if at+3<data.len() {at} else {data.len()};
        if data[from]&31==15 {return Err(invalid("AVC subset SPS is outside the screen contract"));}
        if data[from]&31!=7 {continue;}
        if end-from>65536 {return Err(invalid("AVC SPS exceeds bounded parameter budget"));}
        let mut rbsp=Vec::with_capacity(end-from);let mut zeros=0;
        for (offset,&byte) in data[from+1..end].iter().enumerate() {
            if zeros>=2 && byte==3 {
                if data.get(from+2+offset).filter(|_|from+2+offset<end).is_none_or(|n|*n>3) {return Err(invalid("invalid AVC emulation prevention"));}
                zeros=0;continue;
            }
            rbsp.push(byte);zeros=if byte==0 {zeros+1} else {0};
        }
        let mut bits=AvcBits {data:&rbsp,at:0};
        let profile=bits.read(8)?;let constraints=bits.read(8)?;let level=bits.read(8)?;
        if bits.ue()?>31 {return Err(invalid("AVC SPS id exceeds Annex A"));}
        if profile==100 {
            if bits.ue()?!=1 || bits.ue()?!=0 || bits.ue()?!=0 {return Err(invalid("screen AVC requires 8-bit 4:2:0"));}
            bits.read(1)?;
            if bits.read(1)?!=0 {
                for i in 0..8 {if bits.read(1)?!=0 {
                    let mut last=8i64;let mut next=8i64;
                    for _ in 0..if i<6 {16} else {64} {
                        if next!=0 {next=(last+bits.se()?+256).rem_euclid(256);}
                        if next!=0 {last=next;}
                    }
                }}
            }
        } else if profile!=66 && profile!=77 {return Err(invalid("unsupported screen AVC SPS profile"));}
        if bits.ue()?>12 {return Err(invalid("AVC frame number exceeds Annex A"));}
        match bits.ue()? {
            0=>{if bits.ue()?>12 {return Err(invalid("AVC POC length exceeds Annex A"));}},
            1=>{bits.read(1)?;bits.se()?;bits.se()?;let count=bits.ue()?;
                if count>256 {return Err(invalid("AVC POC cycle exceeds budget"));}
                for _ in 0..count {bits.se()?;}
            },
            2=>{},_=>return Err(invalid("invalid AVC picture order type")),
        }
        let refs=bits.ue()?;
        if refs>16 {return Err(invalid("AVC reference count exceeds Annex A"));}
        bits.read(1)?;
        let width=bits.ue()?.checked_add(1).and_then(|n|n.checked_mul(16)).ok_or_else(||invalid("AVC width overflow"))?;
        let height=bits.ue()?.checked_add(1).and_then(|n|n.checked_mul(16)).ok_or_else(||invalid("AVC height overflow"))?;
        if bits.read(1)?!=1 {return Err(invalid("interlaced AVC is outside the screen contract"));}
        bits.read(1)?;
        let mut crop=[0u32;4];
        if bits.read(1)?!=0 {for value in &mut crop {*value=bits.ue()?;}}
        let visible_w=width.checked_sub(crop[0].checked_add(crop[1]).and_then(|n|n.checked_mul(2)).ok_or_else(||invalid("AVC crop overflow"))?);
        let visible_h=height.checked_sub(crop[2].checked_add(crop[3]).and_then(|n|n.checked_mul(2)).ok_or_else(||invalid("AVC crop overflow"))?);
        let coded=FrameSize::new(width,height);
        let level=if level==11 && constraints&0x10!=0 && (profile==66 || profile==77) {9} else {level};
        let level=avc_limits(level).ok_or_else(||invalid("unknown AVC level"))?;
        if visible_w!=Some(size.width) || visible_h!=Some(size.height) || !level.supports(coded,0) {
            return Err(invalid("AVC SPS dimensions or level contradict the frame contract"));
        }
        // Coded padding also consumes decoder memory, even when it is cropped away.
        limits.check(coded)?;
    }
    Ok(())
}
struct AvcBits<'a> {data:&'a [u8],at:usize}
impl AvcBits<'_> {
    fn read(&mut self,count:usize)->io::Result<u32> {
        if count>32 || self.at.checked_add(count).is_none_or(|n|n>self.data.len()*8) {return Err(invalid("truncated AVC SPS"));}
        let mut value=0;
        for _ in 0..count {value=(value<<1)|u32::from((self.data[self.at/8]>>(7-self.at%8))&1);self.at+=1;}
        Ok(value)
    }
    fn ue(&mut self)->io::Result<u32> {
        let mut zeros=0;
        while self.read(1)?==0 {zeros+=1;if zeros>30 {return Err(invalid("AVC Exp-Golomb overflow"));}}
        Ok((1u32<<zeros)-1+self.read(zeros)?)
    }
    fn se(&mut self)->io::Result<i64> {let n=i64::from(self.ue()?);Ok(if n&1==0 {-n/2} else {(n+1)/2})}
}
