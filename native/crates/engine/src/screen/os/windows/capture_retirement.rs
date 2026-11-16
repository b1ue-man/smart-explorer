//! A failed WGC Close keeps the native objects and their credit alive.
//! The pool reports every delivered picture (`FrameArrived`, plan 2026-11-14
//! bildkette B-10) and the session takes the frame interval
//! (`MinUpdateInterval`, B-11) where Windows has it.
use std::io;
use std::sync::{Arc, Condvar, Mutex, PoisonError};
use std::time::Duration;
use windows::Foundation::{EventRegistrationToken, TimeSpan, TypedEventHandler};
use windows::Graphics::Capture::{
    Direct3D11CaptureFramePool, GraphicsCaptureItem, GraphicsCaptureSession,
};
use windows::Graphics::DirectX::Direct3D11::IDirect3DDevice;
use windows::Graphics::DirectX::DirectXPixelFormat;
use windows::Graphics::SizeInt32;
use windows::Win32::Foundation::RO_E_CLOSED;

use crate::screen::core::capture_admission::CaptureSlice;

/// Pictures the compositor delivered into a pool.
#[derive(Default)]
pub(super) struct Arrivals {
    count: Mutex<u64>,
    signal: Condvar,
}

impl Arrivals {
    /// The number of pictures delivered so far.
    pub(super) fn seen(&self) -> u64 {
        *self.count.lock().unwrap_or_else(PoisonError::into_inner)
    }

    fn arrived(&self) {
        let mut count = self.count.lock().unwrap_or_else(PoisonError::into_inner);
        *count = count.wrapping_add(1);
        self.signal.notify_all();
    }

    /// Waits until a picture arrived after `seen` pictures, at most `timeout`.
    pub(super) fn wait_after(&self, seen: u64, timeout: Duration) {
        let count = self.count.lock().unwrap_or_else(PoisonError::into_inner);
        let _ = self
            .signal
            .wait_timeout_while(count, timeout, |count| *count == seen);
    }
}

/// `MinUpdateInterval` in the 100-ns ticks of a `TimeSpan`.
pub(super) fn time_span(interval: Duration) -> TimeSpan {
    TimeSpan {
        Duration: i64::try_from(interval.as_nanos() / 100).unwrap_or(i64::MAX),
    }
}

struct Pending {
    session: Option<GraphicsCaptureSession>,
    pool: Option<Direct3D11CaptureFramePool>,
    arrived: Option<EventRegistrationToken>,
    // Every native interface is released before its product memory credit.
    _slice: CaptureSlice,
}

fn closed(result: windows::core::Result<()>) -> io::Result<()> {
    match result {
        Ok(()) => Ok(()),
        Err(error) if error.code() == RO_E_CLOSED => Ok(()),
        Err(error) => Err(io::Error::other(error)),
    }
}

impl Pending {
    fn close(&mut self) -> io::Result<()> {
        if let Some(session) = &self.session {
            closed(session.Close())?;
            self.session = None;
        }
        if let Some(pool) = &self.pool {
            if let Some(token) = self.arrived.take() {
                // A pool that refuses the removal is closed right after.
                let _ = pool.RemoveFrameArrived(token);
            }
            closed(pool.Close())?;
            self.pool = None;
        }
        Ok(())
    }
}

pub(super) struct CaptureResources {
    pending: Option<Pending>,
    arrivals: Arc<Arrivals>,
}

impl CaptureResources {
    pub(super) fn create(
        device: &IDirect3DDevice,
        item: &GraphicsCaptureItem,
        size: SizeInt32,
        slice: CaptureSlice,
    ) -> io::Result<Self> {
        let pool = Direct3D11CaptureFramePool::CreateFreeThreaded(
            device,
            DirectXPixelFormat::B8G8R8A8UIntNormalized,
            2,
            size,
        )
        .map_err(io::Error::other)?;
        // Partial construction has the same cleanup owner as normal capture.
        let mut resources = Self {
            pending: Some(Pending {
                session: None,
                pool: Some(pool),
                arrived: None,
                _slice: slice,
            }),
            arrivals: Arc::new(Arrivals::default()),
        };
        // The free-threaded pool raises FrameArrived on a system thread; it
        // only counts and wakes the capture thread.
        let signal = resources.arrivals.clone();
        let token = resources
            .pool()?
            .FrameArrived(&TypedEventHandler::<
                Direct3D11CaptureFramePool,
                windows::core::IInspectable,
            >::new(move |_, _| {
                signal.arrived();
                Ok(())
            }))
            .map_err(io::Error::other)?;
        if let Some(pending) = &mut resources.pending {
            pending.arrived = Some(token);
        }
        let session = resources
            .pool()?
            .CreateCaptureSession(item)
            .map_err(io::Error::other)?;
        if let Some(pending) = &mut resources.pending {
            pending.session = Some(session.clone());
        }
        // The pointer stays in the picture unless the viewer draws it itself
        // (overlay, plan 2026-11-14 P6): `attach` sets the session's pointer
        // capture and lets the overlay switch it.
        super::cursor_overlay::attach(&session);
        session.StartCapture().map_err(io::Error::other)?;
        Ok(resources)
    }

    pub(super) fn pool(&self) -> io::Result<&Direct3D11CaptureFramePool> {
        self.pending
            .as_ref()
            .and_then(|pending| pending.pool.as_ref())
            .ok_or_else(|| io::Error::other("WGC frame pool already closed"))
    }

    pub(super) fn arrivals(&self) -> &Arc<Arrivals> {
        &self.arrivals
    }

    /// Sets the session's `MinUpdateInterval` (`IGraphicsCaptureSession5`);
    /// false where Windows lacks the property or refuses the value.
    pub(super) fn set_min_update_interval(&self, interval: Duration) -> bool {
        self.pending
            .as_ref()
            .and_then(|pending| pending.session.as_ref())
            .is_some_and(|session| session.SetMinUpdateInterval(time_span(interval)).is_ok())
    }

    pub(super) fn close(mut self) -> io::Result<()> {
        if let Some(pending) = &mut self.pending {
            pending.close()?;
        }
        self.pending = None;
        Ok(())
    }
}

impl Drop for CaptureResources {
    fn drop(&mut self) {
        let Some(mut pending) = self.pending.take() else {
            return;
        };
        if pending.close().is_ok() {
            return;
        }
        // An error is not proof of release. One owner retries this resource;
        // it never admits replacement bytes or treats elapsed time as an exit.
        let held = Arc::new(Mutex::new(Some(pending)));
        let worker = held.clone();
        let started = std::thread::Builder::new()
            .name("screen-wgc-retirement".into())
            .spawn(move || {
                super::mf_common::ensure_com();
                loop {
                    let mut guard = worker
                        .lock()
                        .unwrap_or_else(std::sync::PoisonError::into_inner);
                    let complete = guard.as_mut().is_none_or(|pending| pending.close().is_ok());
                    if complete {
                        drop(guard.take());
                        return;
                    }
                    drop(guard);
                    std::thread::sleep(Duration::from_millis(250));
                }
            });
        if let Err(error) = started {
            eprintln!(
                "WGC cleanup could not start; capture credit retained until process exit: {error}"
            );
            // Even a thread creation failure cannot turn unknown native
            // ownership into free credit. The parent confirms process exit.
            std::mem::forget(held);
        }
    }
}
