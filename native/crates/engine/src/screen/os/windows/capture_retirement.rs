//! A failed WGC Close keeps the native objects and their credit alive.
use std::io;
use std::sync::{Arc, Mutex};
use std::time::Duration;
use windows::Graphics::Capture::{
    Direct3D11CaptureFramePool, GraphicsCaptureItem, GraphicsCaptureSession,
};
use windows::Graphics::DirectX::Direct3D11::IDirect3DDevice;
use windows::Graphics::DirectX::DirectXPixelFormat;
use windows::Graphics::SizeInt32;
use windows::Win32::Foundation::RO_E_CLOSED;

use crate::screen::core::capture_admission::CaptureSlice;

struct Pending {
    session: Option<GraphicsCaptureSession>,
    pool: Option<Direct3D11CaptureFramePool>,
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
            closed(pool.Close())?;
            self.pool = None;
        }
        Ok(())
    }
}

pub(super) struct CaptureResources {
    pending: Option<Pending>,
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
                _slice: slice,
            }),
        };
        let session = resources
            .pool()?
            .CreateCaptureSession(item)
            .map_err(io::Error::other)?;
        if let Some(pending) = &mut resources.pending {
            pending.session = Some(session.clone());
        }
        let _ = session.SetIsCursorCaptureEnabled(true);
        session.StartCapture().map_err(io::Error::other)?;
        Ok(resources)
    }

    pub(super) fn pool(&self) -> io::Result<&Direct3D11CaptureFramePool> {
        self.pending
            .as_ref()
            .and_then(|pending| pending.pool.as_ref())
            .ok_or_else(|| io::Error::other("WGC frame pool already closed"))
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
