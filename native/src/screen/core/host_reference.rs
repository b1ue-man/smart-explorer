//! Source identity and bounded proof that a current-reference frame reached presentation.
#[cfg(test)]
#[path = "batches_reference_task_tests.rs"]
mod batches_task_tests;

use super::*;
use std::collections::VecDeque;

/// Metadata only, bounded by the existing 64-frame control inbox. Evicted
/// presentation reports cannot unlock input; a subsequent current report can.
const PRESENTATION_HISTORY:usize=64;
pub(super) struct ReferencePresentation {
    floor:u64,
    gop:Option<u64>,
    sent:VecDeque<u64>,
    ready:bool,
}
impl ReferencePresentation {
    pub(super) fn new()->Self {Self {floor:1,gop:None,sent:VecDeque::new(),ready:false}}
    pub(super) fn invalidate(&mut self,floor:u64) {self.floor=floor;self.gop=None;self.sent.clear();self.ready=false;}
    pub(super) fn sent(&mut self,gop:u64,frame:u64,keyframe:bool) {
        if self.gop!=Some(gop) {
            if !keyframe {return;}
            self.gop=Some(gop);self.sent.clear();self.ready=false;
        }
        if frame<self.floor {return;}
        if self.sent.len()==PRESENTATION_HISTORY {self.sent.pop_front();}
        self.sent.push_back(frame);
    }
    fn presented(&mut self,gop:u64,frame:u64)->bool {
        if self.gop!=Some(gop) || !self.sent.contains(&frame) {return false;}
        self.ready=true;
        true
    }
}
impl Pipeline {
    pub(crate) fn presentation_supported(&self)->bool {self.negotiated_contract.is_some_and(|c|c.version==crate::screen::core::frame_budget::FRAME_PROTOCOL)}
    fn source_reference_current(&self)->bool {
        self.capture_active && self.capture_transition.is_none() && self.stage.as_ref().is_some_and(|stage|
            stage.source.reference_generation()==self.source_generation && stage.source.size()==self.source_size
            && (!self.frame_contract.is_some_and(|c|c.hybrid_regions) || stage.source.supports_frame_regions()))
    }
    pub(crate) fn input_reference_ready(&self)->bool {!self.presentation_supported() || (self.presentation.ready && self.source_reference_current())}
    pub(crate) fn input_reference_generation(&self)->u64 {self.source_generation}
    pub(crate) fn reference_floor(&self)->u64 {self.presentation.floor}
    pub(crate) fn on_presented(&mut self,gop:u64,frame:u64)->bool {
        self.presentation_supported() && self.source_reference_current()
            && self.presentation.presented(gop,frame)
    }
    /// Also called after paused/unchanged capture and snapshots, before consuming their result.
    pub(super) fn sync_source_reference(&mut self)->bool {
        let Some(stage)=self.stage.as_ref() else {return false;};
        let (size,generation)=(stage.source.size(),stage.source.reference_generation());
        let contract=self.negotiated_contract.map(|mut c| {c.hybrid_regions &= stage.source.supports_frame_regions();c});
        if size==self.source_size && generation==self.source_generation && contract==self.frame_contract {return false;}
        self.frame_contract=contract;
        self.source_size=size;self.source_generation=generation;self.visible_rects=None;
        self.quality.size=self.fitted_size();self.invalidate_reference();true
    }
}
