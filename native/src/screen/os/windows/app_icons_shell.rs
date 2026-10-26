//! Shell-item bitmap conversion; the caller retains its original COM apartment.
use super::*;

pub(super) fn shell_bitmap(item: &IShellItem, app_id: &str) -> Option<IconReply> {
    let factory: IShellItemImageFactory = item.cast().ok()?;
    let bitmap = unsafe {
        factory
            .GetImage(
                SIZE {
                    cx: SIDE as i32,
                    cy: SIDE as i32,
                },
                SIIGBF_ICONONLY,
            )
            .ok()?
    };
    let result = (|| {
        let mut facts = BitmapFacts {
            kind: 0,
            width: 0,
            height: 0,
            stride: 0,
            planes: 0,
            bits: 0,
            data: std::ptr::null_mut(),
        };
        if unsafe {
            GetObjectW(
                bitmap.0,
                std::mem::size_of::<BitmapFacts>() as i32,
                &mut facts as *mut _ as *mut c_void,
            )
        } == 0
        {
            return None;
        }
        let width = u32::try_from(facts.width).ok()?;
        let height = u32::try_from(facts.height).ok()?;
        let pixels = crate::screen::core::app_icon::checked_pixels(width, height).ok()?;
        let mut info = BitmapInfo {
            size: 40,
            width: facts.width,
            height: -facts.height,
            planes: 1,
            bit_count: 32,
            compression: 0,
            image_size: 0,
            x_pixels: 0,
            y_pixels: 0,
            colors_used: 0,
            colors_important: 0,
            color: 0,
        };
        let dc = unsafe { CreateCompatibleDC(std::ptr::null_mut()) };
        if dc.is_null() {
            return None;
        }
        let mut bgra = vec![0u8; pixels * 4];
        let rows = unsafe {
            GetDIBits(
                dc,
                bitmap.0,
                0,
                height,
                bgra.as_mut_ptr() as RawHandle,
                &mut info,
                0,
            )
        };
        unsafe {
            DeleteDC(dc);
        }
        if rows != facts.height {
            return None;
        }
        let has_alpha = bgra.chunks_exact(4).any(|pixel| pixel[3] != 0);
        let mut rgba = Vec::with_capacity(bgra.len());
        for pixel in bgra.chunks_exact(4) {
            let alpha = if has_alpha { u32::from(pixel[3]) } else { 255 };
            let straight = |channel| {
                if alpha == 0 {
                    0
                } else {
                    ((u32::from(pixel[channel]) * 255 + alpha / 2) / alpha).min(255) as u8
                }
            };
            rgba.extend_from_slice(&[straight(2), straight(1), straight(0), alpha as u8]);
        }
        let png = super::super::app_icon_raster::rgba_png(width, height, &rgba).ok()?;
        IconReply::new(app_id, &png).ok()
    })();
    unsafe {
        DeleteObject(bitmap.0);
    }
    result
}
