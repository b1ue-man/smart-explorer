use super::{IconKind, IconResult};
use std::ffi::c_void;
use windows::core::PCWSTR;
use windows::Win32::Graphics::Gdi::{
    DeleteObject, GetDC, GetDIBits, GetObjectW, ReleaseDC, BITMAP, BITMAPINFO, BITMAPINFOHEADER,
    DIB_RGB_COLORS, HGDIOBJ,
};
use windows::Win32::Storage::FileSystem::{FILE_ATTRIBUTE_DIRECTORY, FILE_ATTRIBUTE_NORMAL};
use windows::Win32::System::Com::{CoInitializeEx, COINIT_MULTITHREADED};
use windows::Win32::UI::Shell::{
    SHGetFileInfoW, SHFILEINFOW, SHGFI_ICON, SHGFI_SMALLICON, SHGFI_USEFILEATTRIBUTES,
};
use windows::Win32::UI::WindowsAndMessaging::{DestroyIcon, GetIconInfo, HICON, ICONINFO};

pub struct IconWorker {
    req_tx: crossbeam_channel::Sender<(String, IconKind)>,
    res_rx: crossbeam_channel::Receiver<IconResult>,
}

impl IconWorker {
    pub fn new() -> Self {
        let (req_tx, req_rx) = crossbeam_channel::unbounded::<(String, IconKind)>();
        let (res_tx, res_rx) = crossbeam_channel::unbounded::<IconResult>();
        std::thread::Builder::new()
            .name("icon-extract".into())
            .spawn(move || worker(req_rx, res_tx))
            .ok();
        Self { req_tx, res_rx }
    }

    pub fn request(&self, key: String, kind: IconKind) {
        let _ = self.req_tx.send((key, kind));
    }

    pub fn drain(&self) -> Vec<IconResult> {
        let mut results = Vec::new();
        while let Ok(r) = self.res_rx.try_recv() {
            results.push(r);
        }
        results
    }
}

fn worker(
    req_rx: crossbeam_channel::Receiver<(String, IconKind)>,
    res_tx: crossbeam_channel::Sender<IconResult>,
) {
    unsafe {
        // Generic type icons resolve fine under MTA; this thread is
        // independent of the STA the shell menu/clipboard init on.
        let _ = CoInitializeEx(None, COINIT_MULTITHREADED);
    }
    while let Ok((key, kind)) = req_rx.recv() {
        let (name, attrs) = match &kind {
            IconKind::Dir => ("folder".to_string(), FILE_ATTRIBUTE_DIRECTORY),
            IconKind::GenericFile => ("file".to_string(), FILE_ATTRIBUTE_NORMAL),
            IconKind::Ext(e) => (format!("file.{}", e), FILE_ATTRIBUTE_NORMAL),
        };
        let (w, h, rgba) = match unsafe { extract(&name, attrs) } {
            Some(v) => v,
            // Send a 1x1 transparent so the cache marks it resolved and we
            // don't re-request the same failing key forever.
            None => (1, 1, vec![0u8; 4]),
        };
        if res_tx.send(IconResult { key, w, h, rgba }).is_err() {
            break;
        }
    }
}

unsafe fn extract(
    name: &str,
    attrs: windows::Win32::Storage::FileSystem::FILE_FLAGS_AND_ATTRIBUTES,
) -> Option<(usize, usize, Vec<u8>)> {
    let wide: Vec<u16> = name.encode_utf16().chain(Some(0)).collect();
    let mut sfi = SHFILEINFOW::default();
    let ret = SHGetFileInfoW(
        PCWSTR(wide.as_ptr()),
        attrs,
        Some(&mut sfi),
        std::mem::size_of::<SHFILEINFOW>() as u32,
        SHGFI_ICON | SHGFI_SMALLICON | SHGFI_USEFILEATTRIBUTES,
    );
    if ret == 0 || sfi.hIcon.is_invalid() {
        return None;
    }
    let out = hicon_to_rgba(sfi.hIcon);
    let _ = DestroyIcon(sfi.hIcon);
    out
}

unsafe fn hicon_to_rgba(hicon: HICON) -> Option<(usize, usize, Vec<u8>)> {
    let mut ii = ICONINFO::default();
    GetIconInfo(hicon, &mut ii).ok()?;
    let hbm_color = ii.hbmColor;
    let hbm_mask = ii.hbmMask;

    let cleanup = || {
        if !hbm_color.is_invalid() {
            let _ = DeleteObject(HGDIOBJ(hbm_color.0));
        }
        if !hbm_mask.is_invalid() {
            let _ = DeleteObject(HGDIOBJ(hbm_mask.0));
        }
    };

    let mut bm = BITMAP::default();
    if GetObjectW(
        HGDIOBJ(hbm_color.0),
        std::mem::size_of::<BITMAP>() as i32,
        Some(&mut bm as *mut _ as *mut c_void),
    ) == 0
    {
        cleanup();
        return None;
    }
    let w = bm.bmWidth.max(0) as usize;
    let h = bm.bmHeight.max(0) as usize;
    if w == 0 || h == 0 {
        cleanup();
        return None;
    }

    let mut bmi = BITMAPINFO::default();
    bmi.bmiHeader.biSize = std::mem::size_of::<BITMAPINFOHEADER>() as u32;
    bmi.bmiHeader.biWidth = w as i32;
    bmi.bmiHeader.biHeight = -(h as i32); // top-down
    bmi.bmiHeader.biPlanes = 1;
    bmi.bmiHeader.biBitCount = 32;
    bmi.bmiHeader.biCompression = 0; // BI_RGB

    let hdc = GetDC(None);
    let mut buf = vec![0u8; w * h * 4];
    let got = GetDIBits(
        hdc,
        hbm_color,
        0,
        h as u32,
        Some(buf.as_mut_ptr() as *mut c_void),
        &mut bmi,
        DIB_RGB_COLORS,
    );
    ReleaseDC(None, hdc);
    if got == 0 {
        cleanup();
        return None;
    }

    // GDI hands back BGRA with straight alpha. Swap to RGBA and notice
    // whether any alpha is present at all.
    let mut any_alpha = false;
    for px in buf.chunks_exact_mut(4) {
        px.swap(0, 2);
        if px[3] != 0 {
            any_alpha = true;
        }
    }

    // Classic icons carry no alpha in the colour bitmap - rebuild it from
    // the AND mask (set bit = transparent).
    if !any_alpha {
        let mut mask = vec![0u8; w * h * 4];
        let hdc2 = GetDC(None);
        let mut bmi2 = bmi;
        let g2 = GetDIBits(
            hdc2,
            hbm_mask,
            0,
            h as u32,
            Some(mask.as_mut_ptr() as *mut c_void),
            &mut bmi2,
            DIB_RGB_COLORS,
        );
        ReleaseDC(None, hdc2);
        if g2 != 0 {
            for (px, m) in buf.chunks_exact_mut(4).zip(mask.chunks_exact(4)) {
                px[3] = if m[0] != 0 { 0 } else { 255 };
            }
        } else {
            for px in buf.chunks_exact_mut(4) {
                px[3] = 255;
            }
        }
    }

    cleanup();
    Some((w, h, buf))
}

#[cfg(test)]
mod tests {
    use super::{worker, IconKind, IconResult};

    /// Exercises the real SHGetFileInfo + GDI extraction path end-to-end on
    /// Windows and asserts we get visible pixels from the worker plumbing.
    #[test]
    fn extracts_real_icons() {
        let (req_tx, req_rx) = crossbeam_channel::unbounded::<(String, IconKind)>();
        let (res_tx, res_rx) = crossbeam_channel::unbounded::<IconResult>();
        std::thread::spawn(move || worker(req_rx, res_tx));

        req_tx.send(("<dir>".into(), IconKind::Dir)).unwrap();
        req_tx
            .send(("txt".into(), IconKind::Ext("txt".into())))
            .unwrap();

        let mut visible = 0;
        for _ in 0..2 {
            let r = res_rx
                .recv_timeout(std::time::Duration::from_secs(10))
                .expect("icon result");
            assert!(r.w >= 1 && r.h >= 1);
            assert_eq!(r.rgba.len(), r.w * r.h * 4);
            if r.rgba.chunks_exact(4).any(|p| p[3] != 0) {
                visible += 1;
            }
        }
        assert!(visible >= 1, "no visible icon pixels were extracted");
        drop(req_tx);
    }
}
