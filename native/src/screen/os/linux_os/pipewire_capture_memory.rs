//! Image-generation admission; native imports own separate exact tickets.
use super::super::spa_pod::VideoFormat;
use crate::screen::core::capture_admission::{
    CaptureGeneration, CaptureGrant, CaptureOwner, SessionAdmission, SourceKey,
};
use crate::screen::core::capture_import::CaptureImportConnection;
use crate::screen::core::capture_plan::{CapturePhases, CapturePlan};
use crate::screen::core::host_codec_adapters::{HostInputPixels, HostOutputPlan};
use crate::screen::core::pixel_convert::FrameSize;
use std::io;
use std::sync::{Arc, Condvar, Mutex, PoisonError};

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct Demand {
    pub generation: CaptureGeneration,
    pub format: VideoFormat,
    pub size: FrameSize,
}
struct Inner {
    generation: CaptureGeneration,
    format: Option<(VideoFormat, FrameSize)>,
    output: Option<HostOutputPlan>,
    prepared: Option<Demand>,
    activation: Option<CaptureGrant>,
    current: Option<CaptureGrant>,
    active: bool,
    stopped: bool,
    revision: u64,
}
pub(crate) struct CaptureMemory {
    source: SourceKey,
    admission: Arc<SessionAdmission>,
    inner: Mutex<Inner>,
    changed: Condvar,
}
impl CaptureMemory {
    pub(crate) fn new(source: SourceKey, admission: Arc<SessionAdmission>) -> Self {
        Self {
            source,
            admission,
            inner: Mutex::new(Inner {
                generation: CaptureGeneration::new(0),
                format: None,
                output: None,
                prepared: None,
                activation: None,
                current: None,
                active: false,
                stopped: false,
                revision: 0,
            }),
            changed: Condvar::new(),
        }
    }
    pub(crate) fn source(&self) -> &SourceKey {
        &self.source
    }
    pub(super) fn import_connection(&self) -> io::Result<CaptureImportConnection> {
        self.admission.import_connection(&self.source)
    }
    pub(crate) fn generation(&self) -> CaptureGeneration {
        self.inner
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .generation
    }
    pub(crate) fn current_grant(&self) -> Option<CaptureGrant> {
        self.inner
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .current
            .clone()
    }
    pub(super) fn authority(
        &self,
    ) -> Arc<dyn crate::screen::core::capture_admission::CaptureAuthority> {
        self.admission.authority()
    }
    pub(super) fn installed(&self, demand: &Demand, grant: CaptureGrant) -> bool {
        let mut inner = self.inner.lock().unwrap_or_else(PoisonError::into_inner);
        if inner.stopped || Self::demand(&inner).as_ref() != Some(demand) {
            let _ = grant.retire();
            return false;
        }
        inner.current = Some(grant);
        true
    }
    pub(crate) fn size(&self) -> Option<FrameSize> {
        self.inner
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .format
            .map(|(_, size)| size)
    }
    pub(super) fn format(&self, value: Option<(VideoFormat, FrameSize)>) -> io::Result<()> {
        let mut inner = self
            .inner
            .lock()
            .map_err(|_| io::Error::other("pipewire admission poisoned"))?;
        inner.generation = inner.generation.next()?;
        inner.format = value;
        inner.prepared = None;
        if let Some(grant) = inner.current.take() {
            grant.retire()?;
        }
        if let Some(grant) = inner.activation.take() {
            grant.retire()?;
        }
        inner.revision = inner.revision.wrapping_add(1);
        self.changed.notify_all();
        Ok(())
    }
    fn demand(inner: &Inner) -> Option<Demand> {
        let (format, size) = inner.format?;
        Some(Demand {
            generation: inner.generation,
            format,
            size,
        })
    }
    pub(super) fn plan_for(
        &self,
        demand: &Demand,
        output: &HostOutputPlan,
    ) -> io::Result<CapturePlan> {
        let mut plan = CapturePlan::new(self.source.clone(), demand.generation);
        plan.add_bgra(
            CaptureOwner::SourcePixels,
            demand.size,
            3,
            CapturePhases::ALL,
        )?;
        let input = if output.size() == demand.size {
            HostInputPixels::BorrowedSource
        } else {
            HostInputPixels::SeparateImage
        };
        output.append_to(&mut plan, input)?;
        Ok(plan)
    }
    pub(crate) fn capture_plan(&self, output: &HostOutputPlan) -> io::Result<Option<CapturePlan>> {
        let mut inner = self
            .inner
            .lock()
            .map_err(|_| io::Error::other("pipewire admission poisoned"))?;
        if inner.stopped {
            return Err(io::Error::other("pipewire preparation closed"));
        }
        let Some(demand) = Self::demand(&inner) else {
            return Ok(None);
        };
        let plan = self.plan_for(&demand, output)?;
        inner.output = Some(output.clone());
        inner.prepared = Some(demand);
        Ok(Some(plan))
    }
    pub(crate) fn activate(&self, grant: CaptureGrant) -> io::Result<()> {
        let mut inner = self
            .inner
            .lock()
            .map_err(|_| io::Error::other("pipewire admission poisoned"))?;
        let demand =
            Self::demand(&inner).ok_or_else(|| io::Error::other("pipewire format pending"))?;
        if inner.stopped || inner.prepared.as_ref() != Some(&demand) {
            return Err(io::Error::other("pipewire prepared format changed"));
        }
        let output = inner
            .output
            .as_ref()
            .ok_or_else(|| io::Error::other("pipewire output plan missing"))?;
        let plan = self.plan_for(&demand, output)?;
        grant.check_identity(&self.source, demand.generation)?;
        if grant.plan().peak_bytes() < plan.peak_bytes()
            || grant
                .plan()
                .owner_limits()
                .iter()
                .zip(plan.owner_limits())
                .any(|(have, need)| *have < need)
        {
            return Err(io::Error::other("pipewire image grant too small"));
        }
        inner.current = Some(grant.clone());
        inner.activation = Some(grant);
        inner.active = true;
        inner.revision = inner.revision.wrapping_add(1);
        self.changed.notify_all();
        Ok(())
    }
    /// Returns Parent work without holding any image or native lock.
    pub(super) fn frame_work(&self, seen: &mut u64) -> io::Result<Option<(Demand, CaptureGrant)>> {
        let mut inner = self
            .inner
            .lock()
            .map_err(|_| io::Error::other("pipewire admission poisoned"))?;
        if inner.stopped || !inner.active || *seen == inner.revision {
            return Ok(None);
        }
        let previous_seen = *seen;
        *seen = inner.revision;
        let Some(demand) = Self::demand(&inner) else {
            return Ok(None);
        };
        if let Some(grant) = inner.activation.take() {
            return Ok(Some((demand, grant)));
        }
        let output = inner
            .output
            .clone()
            .ok_or_else(|| io::Error::other("pipewire output plan missing"))?;
        drop(inner);
        let plan = self.plan_for(&demand, &output)?;
        match self.authority().approve(plan) {
            Ok(grant) => Ok(Some((demand, grant))),
            // Old pictures/transports retain their real credit until their
            // final owner ends. Retry this same demand after the normal wait.
            Err(error) if error.kind() == io::ErrorKind::WouldBlock => {
                *seen = previous_seen;
                Ok(None)
            }
            Err(error) => Err(error),
        }
    }
    pub(super) fn stopped(&self) -> bool {
        self.inner
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .stopped
    }
    /// Native callbacks only close the local gate and wake the portal owner.
    pub(super) fn signal_stop(&self) {
        self.inner
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .stopped = true;
        self.changed.notify_all();
    }
    pub(super) fn stop(&self) {
        let (current, activation) = {
            let mut inner = self.inner.lock().unwrap_or_else(PoisonError::into_inner);
            inner.stopped = true;
            (inner.current.take(), inner.activation.take())
        };
        for grant in [current, activation].into_iter().flatten() {
            let _ = grant.retire();
        }
        self.admission.cancel_pending_capture(&self.source);
        self.changed.notify_all();
    }
    pub(super) fn wake(&self) {
        self.changed.notify_all();
    }
    pub(super) fn wait(&self) {
        let inner = self.inner.lock().unwrap_or_else(PoisonError::into_inner);
        if !inner.stopped {
            drop(
                self.changed
                    .wait_timeout(inner, std::time::Duration::from_millis(100)),
            );
        }
    }
    #[cfg(test)]
    pub(crate) fn activate_for_test(&self) -> io::Result<()> {
        use crate::screen::core::host_codec_adapters::{HostCodecPlan, HostContract};
        if self.current_grant().is_some() {
            return Ok(());
        }
        let Some(size) = self.size() else {
            return Ok(());
        };
        let output = HostOutputPlan::new(
            HostContract::Legacy,
            crate::screen::core::frame_budget::FrameLimits::LEGACY.fit(size),
            HostCodecPlan::Tiles,
        )?;
        if let Some(plan) = self.capture_plan(&output)? {
            self.activate(self.authority().approve(plan)?)?;
        }
        Ok(())
    }
}
#[cfg(test)]
pub(crate) fn convert_rows(
    src: &[u8],
    stride: usize,
    format: VideoFormat,
    size: FrameSize,
) -> io::Result<crate::screen::core::pixel_convert::BgraImage> {
    let row = size.width as usize * 4;
    let needed = super::frame_copy::row_span(stride, size)?;
    if src.len() < needed {
        return Err(io::Error::other("pipewire buffer smaller than picture"));
    }
    let mut image = crate::screen::core::pixel_convert::BgraImage::new(size)?;
    for (y, source) in src.chunks(stride).take(size.height as usize).enumerate() {
        let target = image.row_mut(y as u32);
        target.copy_from_slice(&source[..row]);
        for pixel in target.chunks_exact_mut(4) {
            if format.swaps_red_blue() {
                pixel.swap(0, 2);
            }
            pixel[3] = 255;
        }
    }
    Ok(image)
}
pub(super) use super::frame_copy::convert_rows_admitted;
#[cfg(test)]
#[path = "batches_capture_pipewire_task_tests.rs"]
mod tests;

#[cfg(test)]
mod turnover_tests {
    use super::*;
    use crate::screen::core::{
        capture_admission::CaptureAuthority,
        capture_image::CaptureImageBuilder,
        capture_test_support,
        frame_budget::FrameLimits,
        host_codec_adapters::{HostCodecPlan, HostContract},
    };
    use std::sync::atomic::{AtomicBool, Ordering};
    struct RetryAuthority {
        parent: Arc<SessionAdmission>,
        block: Arc<AtomicBool>,
    }
    impl CaptureAuthority for RetryAuthority {
        fn approve(&self, plan: CapturePlan) -> io::Result<CaptureGrant> {
            if self.block.swap(false, Ordering::SeqCst) {
                return Err(io::Error::new(
                    io::ErrorKind::WouldBlock,
                    "old reply still owns its credit",
                ));
            }
            self.parent.authority().approve(plan)
        }
    }
    #[test]
    fn batches_task_capture_linux_pipewire_retries_the_same_format_after_live_credit_pressure() {
        let parent = capture_test_support::session(None);
        let block = Arc::new(AtomicBool::new(false));
        let delegated = Arc::new(
            SessionAdmission::from_verified_parent(
                parent.session(),
                parent.working_bytes(),
                None,
                Arc::new(RetryAuthority {
                    parent: parent.clone(),
                    block: block.clone(),
                }),
                parent.clone(),
            )
            .unwrap(),
        );
        let source = SourceKey::new(parent.session(), "pipewire-retry").unwrap();
        let memory = CaptureMemory::new(source, delegated);
        let size = FrameSize::new(4, 3);
        memory.format(Some((VideoFormat::Bgra, size))).unwrap();
        let output = HostOutputPlan::new(HostContract::Legacy, size, HostCodecPlan::Tiles).unwrap();
        let old = memory
            .authority()
            .approve(memory.capture_plan(&output).unwrap().unwrap())
            .unwrap();
        memory.activate(old.clone()).unwrap();
        let image = CaptureImageBuilder::new(
            size,
            FrameLimits::RESOURCE,
            &old,
            CaptureOwner::SourcePixels,
        )
        .unwrap()
        .freeze();
        memory
            .format(Some((VideoFormat::Bgra, FrameSize::new(8, 3))))
            .unwrap();
        assert!(old.is_retired().unwrap());
        assert_eq!(old.live_bytes().unwrap(), 48);
        block.store(true, Ordering::SeqCst);
        let mut seen = 0;
        assert!(memory.frame_work(&mut seen).unwrap().is_none());
        assert_eq!(
            seen, 0,
            "retryable pressure must not consume the new format revision"
        );
        let (demand, grant) = memory.frame_work(&mut seen).unwrap().unwrap();
        assert_eq!(demand.generation, memory.generation());
        assert_eq!(demand.size, FrameSize::new(8, 3));
        assert!(memory.installed(&demand, grant));
        assert!(memory.frame_work(&mut seen).unwrap().is_none());
        drop(image);
        assert_eq!(old.live_bytes().unwrap(), 0);
        memory.stop();
    }
}
