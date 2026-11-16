//! The DMA-BUF route of the hardware encoder (BK8b, B-15/B-18; joined to
//! the host pipeline by merge W2): a GPU picture of the PipeWire capture
//! (`hw_dmabuf_picture::DmaBufPicture`) is imported, converted and scaled
//! by the VA video processor straight into the session's input; no CPU
//! copy. Every other picture takes the CPU route as before. Failures of the
//! GPU route end DMA-BUF for the session (`fall_back_from_worker` of the
//! stream) and skip only this picture; the encoder stays, the next picture
//! comes as shared memory. An encoder without import (NVENC, a VA session
//! without video processor) reads the picture back and asks the stream for
//! shared memory (`Consumer`).
use std::io;

use super::super::hw_dmabuf_picture::{cause_of, DmaBufPicture};
use super::*;
use crate::screen::core::capture_picture::encode_on_cpu;

impl<S: HwH264Session> HwH264Encoder<S> {
    /// Whether this encoder takes the DMA-BUF capture (BK8b): only then the
    /// stream may keep DMA-BUF, else it falls back to shared memory.
    pub(in crate::screen::os) fn takes_dmabuf(&self) -> bool {
        self.grant.is_some() && self.session.imports_dmabuf()
    }

    /// `ScreenEncoder::encode_picture`: the GPU route for a DMA-BUF picture
    /// this encoder imports, else the CPU pixels (a GPU picture only read
    /// back, `encode_on_cpu`).
    pub(super) fn encode_gpu_picture(
        &mut self,
        picture: &Picture,
        request: &StageRequest,
    ) -> io::Result<CapturedEncodedFrame> {
        let found = match picture {
            Picture::Gpu(gpu) => {
                if let Some(own) = &self.grant {
                    own.check_identity(gpu.source(), gpu.generation())?;
                }
                gpu.surface()
                    .as_any()
                    .downcast_ref::<DmaBufPicture>()
                    .map(|dmabuf| (dmabuf, gpu.grant()))
            }
            Picture::Cpu(_) => None,
        };
        let Some((dmabuf, grant)) = found else {
            return encode_on_cpu(self, picture, request);
        };
        if !self.takes_dmabuf() {
            // The read-back of `encode_on_cpu` (`GpuSurface::read_back`) asks
            // the stream for shared memory (`DmaBufCause::Consumer`).
            return encode_on_cpu(self, picture, request);
        }
        let held = dmabuf.take_held();
        if held.is_none() && !self.has_input {
            // A key frame of a still screen for an encoder without a loaded
            // input surface: this one picture is read back once.
            let image = dmabuf.read_once(request.size, &grant)?;
            return self.encode_captured(&image, request);
        }
        self.encode_dmabuf(held, request, dmabuf)
    }

    /// Encodes a DMA-BUF picture of the capture (BK8b, B-15/B-18) without a
    /// CPU copy: the GPU converts and scales it into the session's input,
    /// then `held` is dropped (the buffer goes back to the compositor) and
    /// the input is encoded. The coded bytes are admitted like every other
    /// encoder output (`CodecOutput`); the input occupies no admitted CPU
    /// memory (the buffer was admitted as a native surface at its import).
    /// `None` for `held` re-encodes the picture already in the input (a key
    /// frame without a new picture). A failed import or conversion leaves
    /// DMA-BUF for the session and skips the picture (`WouldBlock`).
    fn encode_dmabuf(
        &mut self,
        held: Option<Box<dyn HeldDmaBuf + Send>>,
        request: &StageRequest,
        stream: &DmaBufPicture,
    ) -> io::Result<CapturedEncodedFrame> {
        let grant = self
            .grant
            .clone()
            .ok_or_else(|| invalid("hardware encoder has no capture grant"))?;
        if request.size != self.size
            || grant.plan().contract_binding() != Some(request.frame_contract)
        {
            return Err(invalid("DMA-BUF request differs from the hardware encoder"));
        }
        match held {
            Some(held) => {
                let (source, generation) = held.identity();
                grant.check_identity(source, generation)?;
                let size = held.frame().size;
                if size.width < self.size.width || size.height < self.size.height {
                    return Err(invalid("DMA-BUF smaller than the encoder picture"));
                }
                self.has_input = false;
                let imported = self.session.import_dmabuf(held.frame());
                drop(held);
                if let Err(error) = imported {
                    stream.fall_back(cause_of(&error));
                    return Err(io::Error::new(
                        io::ErrorKind::WouldBlock,
                        format!("DMA-BUF import: {error}"),
                    ));
                }
                self.has_input = true;
            }
            None if !self.has_input => {
                return Err(invalid("no DMA-BUF picture to repeat"));
            }
            None => {}
        }
        let packet = crate::screen::core::frame_size_caps::PacketBudget::for_contract(
            request.frame_contract,
        )?;
        let maximum = packet.record_bytes.min(MAX_RECORD_BYTES);
        let mut make =
            |length: usize| AdmittedBytes::buffer(&grant, CaptureOwner::CodecOutput, length as u64);
        let (bytes, keyframe) = self.run(
            HwInput::Prepared,
            (request.keyframe, request.frame_id),
            request.target_bps,
            maximum,
            &mut make,
        )?;
        Ok(CapturedEncodedFrame {
            records: CapturedRecords::one(bytes)?,
            keyframe,
        })
    }
}
