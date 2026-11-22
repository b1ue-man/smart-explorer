//! The NV12 input of the Linux NVENC session from the CPU's I420 (split
//! from `nvenc_encode.rs`, review 2026-11-19 round 9).
use super::*;

impl NvencSession {
    /// Writes I420 planes as NV12 into the input buffer.
    pub(super) fn upload(
        &mut self,
        yuv: &crate::screen::core::pixel_convert::I420Image,
    ) -> io::Result<u32> {
        let f = &self.api.functions;
        let mut lock = NvEncLockInputBuffer::zeroed();
        lock.version = NV_ENC_LOCK_INPUT_BUFFER_VER;
        lock.input_buffer = self.input;
        let lock_input = f.lock_input_buffer.ok_or_else(missing)?;
        // SAFETY: valid session and input buffer.
        let result = unsafe { lock_input(self.encoder, &mut lock) };
        status(&self.api, self.encoder, result, "nvEncLockInputBuffer")?;
        let pitch = lock.pitch as usize;
        let (width, height) = (self.size.width as usize, self.size.height as usize);
        let chroma = self.size.chroma();
        let (chroma_width, rows) = (chroma.width as usize, chroma.height as usize);
        let fits = pitch >= chroma_width * 2
            && pitch >= width
            && yuv.y.len() >= width * height
            && yuv.u.len() >= chroma_width * rows
            && yuv.v.len() >= chroma_width * rows;
        if fits && !lock.buffer_data_ptr.is_null() {
            // SAFETY: the locked NV12 buffer of `pitch` × (height × 3 / 2)
            // bytes (luma rows, then the interleaved chroma rows at
            // `pitch × height`, the NVENC NV12 layout) is ours until unlocked.
            let target = unsafe {
                std::slice::from_raw_parts_mut(
                    lock.buffer_data_ptr.cast::<u8>(),
                    pitch * (height + rows),
                )
            };
            for row in 0..height {
                target[row * pitch..][..width].copy_from_slice(&yuv.y[row * width..][..width]);
            }
            for row in 0..rows {
                let line = &mut target[(height + row) * pitch..][..chroma_width * 2];
                let u = &yuv.u[row * chroma_width..][..chroma_width];
                let v = &yuv.v[row * chroma_width..][..chroma_width];
                for (pair, (u, v)) in line.chunks_exact_mut(2).zip(u.iter().zip(v)) {
                    pair[0] = *u;
                    pair[1] = *v;
                }
            }
        }
        let unlock = f.unlock_input_buffer.ok_or_else(missing)?;
        // SAFETY: the buffer was locked above.
        let result = unsafe { unlock(self.encoder, self.input) };
        status(&self.api, self.encoder, result, "nvEncUnlockInputBuffer")?;
        if !fits || lock.buffer_data_ptr.is_null() {
            return Err(invalid("NVENC input buffer smaller than the picture"));
        }
        Ok(lock.pitch)
    }
}
