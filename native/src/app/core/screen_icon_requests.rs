//! Nonblocking, bounded icon demand from the rows actually rendered by the viewer.
use std::collections::{HashMap,HashSet};
use std::sync::Arc;
use std::time::{Duration,Instant};
use crate::screen::core::app_icon::{IconRequest,IconResponse};

pub(crate) type RequestIcon=Arc<dyn Fn(IconRequest)+Send+Sync>;
#[derive(Default)]
pub(crate) struct IconRequests {
    sender:Option<RequestIcon>,
    enabled:bool,
    pending:HashMap<IconRequest,Instant>,
    missing:HashSet<IconRequest>,
}

impl IconRequests {
    pub(crate) fn set_sender(&mut self,sender:RequestIcon) {self.sender=Some(sender);}
    pub(crate) fn negotiate(&mut self,enabled:bool) {self.enabled=enabled;}
    pub(crate) fn request(&mut self,request:IconRequest) {
        if !self.enabled || request.validate().is_err() || self.missing.contains(&request) {return;}
        self.pending.retain(|_,at|at.elapsed()<Duration::from_secs(30));
        if self.pending.len()>=8 || self.pending.contains_key(&request) {return;}
        let Some(sender)=&self.sender else {return;};
        self.pending.insert(request.clone(),Instant::now());
        sender(request);
    }
    pub(crate) fn receive(&mut self,response:&IconResponse)->bool {
        if self.pending.remove(&response.request).is_none() || response.validate().is_err() {return false;}
        if response.content.is_none() {
            if self.missing.len()>=1024 {self.missing.clear();}
            self.missing.insert(response.request.clone());
        }
        true
    }
    pub(crate) fn refreshed(&mut self) {self.missing.clear();}
}
