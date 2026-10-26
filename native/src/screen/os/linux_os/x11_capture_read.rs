//! Grant-owned X11 SHM/GetImage reads and bounded named-pixmap lifetime.
use super::{
    failed, lost, reply_error, AdmittedBytes, CaptureImageBuilder, CaptureOwner, FrameLimits,
    FrameSize, ImageFormat, Segment, Window, X11Source,
};
use std::io;
use x11rb::connection::Connection;
use x11rb::protocol::composite::ConnectionExt as _;
use x11rb::protocol::shm::ConnectionExt as _;
use x11rb::protocol::xproto::ConnectionExt as _;

impl X11Source {
    fn ensure_segment(&mut self, len: usize) -> io::Result<()> {
        if self
            .segment
            .as_ref()
            .is_some_and(|segment| segment.len == len)
        {
            return Ok(());
        }
        if let Some(old) = self.segment.take() {
            old.release(&self.conn);
        }
        self.segment = Some(Segment::create(
            &self.conn,
            len,
            self.grant
                .as_ref()
                .ok_or_else(|| failed("capture_not_activated"))?,
        )?);
        Ok(())
    }

    /// The bytes of the rectangle (`x`, `y`, `size`) of `drawable` as
    /// 32-bit pixels with opaque alpha.
    pub(super) fn read(
        &mut self,
        drawable: u32,
        x: i16,
        y: i16,
        size: FrameSize,
        window: bool,
    ) -> io::Result<CaptureImageBuilder> {
        let len = FrameLimits::RESOURCE.check(size)?;
        let grant = self
            .grant
            .clone()
            .ok_or_else(|| failed("capture_not_activated"))?;
        let (width, height) = (size.width as u16, size.height as u16);
        let bytes = if self.shm {
            self.ensure_segment(len)?;
            let mut bytes = AdmittedBytes::new(&grant, CaptureOwner::SourcePixels, len as u64)?;
            let segment = self
                .segment
                .as_ref()
                .ok_or_else(|| failed("missing SHM segment"))?;
            self.conn
                .shm_get_image(
                    drawable,
                    x,
                    y,
                    width,
                    height,
                    !0,
                    u8::from(ImageFormat::Z_PIXMAP),
                    segment.id,
                    0,
                )
                .map_err(lost)?
                .reply()
                .map_err(|error| reply_error(error, window))?;
            bytes
                .as_mut_slice()
                .copy_from_slice(&segment.bytes()[..len]);
            bytes
        } else {
            // The closure claims parsed reply storage before x11rb's to_vec.
            // The stream separately holds its raw PacketReader promise.
            let bytes = AdmittedBytes::from_allocator(
                &grant,
                CaptureOwner::SourcePixels,
                len as u64,
                || {
                    let fence = self.conn.get_input_focus().map_err(lost)?;
                    let sequence = fence
                        .sequence_number()
                        .checked_add(1)
                        .ok_or_else(|| failed("x11 sequence exhausted"))?;
                    fence.reply().map_err(|error| reply_error(error, window))?;
                    self.gate.image(sequence, len as u64, &grant)?;
                    let cookie = self
                        .conn
                        .get_image(ImageFormat::Z_PIXMAP, drawable, x, y, width, height, !0)
                        .map_err(lost)?;
                    if cookie.sequence_number() != sequence {
                        return Err(failed("x11 image sequence changed"));
                    }
                    let reply = cookie.reply().map_err(|error| reply_error(error, window))?;
                    self.gate.finish()?;
                    if reply.data.len() != len {
                        return Err(failed("short image"));
                    }
                    Ok(reply.data)
                },
            )?;
            bytes
        };
        let mut image =
            CaptureImageBuilder::from_bytes(size, FrameLimits::RESOURCE, &grant, bytes)?;
        for pixel in image.pixels_mut().chunks_exact_mut(4) {
            pixel[3] = 255;
        }
        Ok(image)
    }

    /// The window's own pixels from the compositor's pixmap.
    pub(super) fn read_pixmap(
        &mut self,
        window: Window,
        size: FrameSize,
    ) -> io::Result<CaptureImageBuilder> {
        // No Parent call while the server is grabbed. A changed border or
        // geometry is rejected here and readmitted on the next pass.
        let expected = self.pixmap_geometry(window)?;
        if expected.1 != size || expected.2 != self.pixmap_bytes {
            return Err(failed("x11_pixmap_geometry_changed"));
        }
        let bytes = expected.2;
        let grant = self
            .grant
            .clone()
            .ok_or_else(|| failed("capture_not_activated"))?;
        let slice = grant.claim(CaptureOwner::NativeSurface, bytes)?;
        self.conn
            .grab_server()
            .map_err(lost)?
            .check()
            .map_err(|e| reply_error(e, true))?;
        let named = (|| {
            if self.pixmap_geometry(window)? != expected {
                return Err(failed("x11_pixmap_geometry_changed"));
            }
            let pixmap = self.conn.generate_id().map_err(failed)?;
            self.conn
                .composite_name_window_pixmap(window, pixmap)
                .map_err(lost)?
                .check()
                .map_err(|e| reply_error(e, true))?;
            Ok(pixmap)
        })();
        let ungrab = self
            .conn
            .ungrab_server()
            .map_err(lost)
            .and_then(|c| c.check().map_err(|e| reply_error(e, true)));
        let pixmap = match named {
            Ok(pixmap) => pixmap,
            Err(error) => {
                if error.kind() == io::ErrorKind::BrokenPipe {
                    self.gate.retain(slice);
                }
                ungrab?;
                return Err(error);
            }
        };
        // The named storage includes borders; read only the original interior.
        let data = match ungrab {
            Ok(()) => self.read(pixmap, expected.0 as i16, expected.0 as i16, size, true),
            Err(error) => Err(error),
        };
        let freed = self
            .conn
            .free_pixmap(pixmap)
            .map_err(lost)
            .and_then(|c| c.check().map_err(|e| reply_error(e, true)));
        if freed.is_err() {
            self.gate.retain(slice);
        } else {
            drop(slice);
        }
        freed?;
        data
    }
}
