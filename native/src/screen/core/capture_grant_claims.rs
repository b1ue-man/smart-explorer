//! Root/member claim accounting and serialized retirement notifications.
use std::io;

use super::super::capture_plan::CaptureOwner;
use super::super::wire::invalid;
use super::{Backing, CaptureGrant, CaptureSlice, GrantInner};

impl Drop for GrantInner {
    fn drop(&mut self) {
        if let Backing::Session(session) = &self.backing {
            let usage = self
                .usage
                .get_mut()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            session.release_plan(self.plan.source(), usage.reserved);
        }
    }
}

impl CaptureGrant {
    pub(crate) fn claim(&self, owner: CaptureOwner, bytes: u64) -> io::Result<CaptureSlice> {
        if self.image_admission().is_some() {
            return Err(invalid("local image copies require an explicit ImageRole claim"));
        }
        if bytes > self.inner.limits[owner as usize]
            || bytes > self.inner.capacity
            || self.member.as_ref().is_some_and(|member| {
                bytes > member.limits[owner as usize] || bytes > member.capacity
            })
        {
            return Err(invalid("single capture claim exceeds its admitted limit"));
        }
        let mut usage = self
            .inner
            .usage
            .lock()
            .map_err(|_| io::Error::other("capture grant poisoned"))?;
        if usage.retired {
            return Err(invalid("retired capture grant cannot authorize new claims"));
        }
        let owner_total = usage.owners[owner as usize]
            .checked_add(bytes)
            .ok_or_else(|| invalid("capture owner claim overflow"))?;
        let total = usage
            .total
            .checked_add(bytes)
            .ok_or_else(|| invalid("capture total claim overflow"))?;
        if owner_total > self.inner.limits[owner as usize] || total > self.inner.capacity {
            return Err(io::Error::new(
                io::ErrorKind::WouldBlock,
                "capture slice exceeds admitted ownership",
            ));
        }
        if let Some(member) = &self.member {
            let mut local = member
                .usage
                .lock()
                .map_err(|_| io::Error::other("capture member grant poisoned"))?;
            let local_owner = local.owners[owner as usize]
                .checked_add(bytes)
                .ok_or_else(|| invalid("capture member owner overflow"))?;
            let local_total = local
                .total
                .checked_add(bytes)
                .ok_or_else(|| invalid("capture member total overflow"))?;
            if local_owner > member.limits[owner as usize] || local_total > member.capacity {
                return Err(io::Error::new(
                    io::ErrorKind::WouldBlock,
                    "capture slice exceeds bound member ownership",
                ));
            }
            local.owners[owner as usize] = local_owner;
            local.total = local_total;
        }
        usage.owners[owner as usize] = owner_total;
        usage.total = total;
        // Each non-Clone value owns a distinct logical credit. Pixel buffers
        // are separate allocations, so imposing arena contiguity would invent
        // fragmentation that the product's allocator does not have.
        Ok(CaptureSlice {
            grant: Some(self.inner.clone()),
            member: self.member.clone(),
            owner,
            bytes,
            image: None,
        })
    }

    /// End capture authority while retaining only its real live slices. All
    /// aliases and family members share this irreversible root transition.
    pub(crate) fn retire(&self) -> io::Result<()> {
        let mut usage = self
            .inner
            .usage
            .lock()
            .map_err(|_| io::Error::other("capture grant poisoned"))?;
        if usage.retired {
            return Ok(());
        }
        if let Backing::Session(session) = &self.inner.backing {
            session.reduce_plan(usage.reserved, usage.total)?;
            usage.reserved = usage.total;
        }
        usage.retired = true;
        drop(usage);
        self.inner.notify_retired();
        Ok(())
    }

    pub(crate) fn live_bytes(&self) -> io::Result<u64> {
        if let Some(admission) = self.image_admission() {
            return admission.live_bytes();
        }
        Ok(self
            .inner
            .usage
            .lock()
            .map_err(|_| io::Error::other("capture grant poisoned"))?
            .total)
    }
}

impl GrantInner {
    pub(super) fn notify_retired(&self) {
        let Backing::Delegated(credit) = &self.backing else {
            return;
        };
        let Some(observer) = &credit.observer else {
            return;
        };
        // Serialize notifications, then sample current ownership so competing
        // drops cannot publish an older, larger snapshot after a newer one.
        // The callback runs outside root/member/session/pool bookkeeping locks.
        let _notification = self
            .notifications
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let usage = self
            .usage
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if !usage.retired {
            return;
        }
        let owners = usage.owners;
        drop(usage);
        observer(owners);
    }
}
