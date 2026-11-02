#[cfg(windows)]
mod windows;
#[cfg(windows)]
pub use windows::WindowsAdapter;
#[cfg(target_os = "linux")]
mod x11;
#[cfg(target_os = "linux")]
pub use x11::X11Adapter;

pub fn adapter() -> std::io::Result<Box<dyn crate::remote_window::WindowAdapter>> {
    #[cfg(windows)]
    {
        return Ok(Box::new(WindowsAdapter::new()?));
    }
    #[cfg(target_os = "linux")]
    {
        return Ok(Box::new(X11Adapter::connect()?));
    }
    #[cfg(not(any(windows, target_os = "linux")))]
    {
        Err(std::io::Error::new(
            std::io::ErrorKind::Unsupported,
            "window_family_requires_native_windows_or_x11",
        ))
    }
}

pub fn adopt(
    pin: &crate::remote_window::WindowPin,
) -> std::io::Result<Box<dyn crate::remote_window::WindowAdapter>> {
    #[cfg(windows)]
    {
        return Ok(Box::new(WindowsAdapter::adopt(pin)?));
    }
    #[cfg(target_os = "linux")]
    {
        return Ok(Box::new(X11Adapter::adopt(pin)?));
    }
    #[cfg(not(any(windows, target_os = "linux")))]
    {
        let _ = pin;
        Err(std::io::Error::new(
            std::io::ErrorKind::Unsupported,
            "window_family_requires_native_windows_or_x11",
        ))
    }
}
#[path = "shared/family_source.rs"]
pub mod family_source;
