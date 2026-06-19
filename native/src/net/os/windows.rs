use std::io;

fn to_wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(std::iter::once(0)).collect()
}

fn wnet_error(code: u32, share: &str) -> io::Error {
    let what = match code {
        5 => "Zugriff verweigert",
        53 => "Netzwerkpfad nicht gefunden",
        67 => "Netzwerkname nicht gefunden",
        86 => "falsches Passwort",
        1219 => "Konflikt mit bestehenden Anmeldedaten für diese Freigabe",
        1326 => "Anmeldung fehlgeschlagen (Benutzer/Passwort)",
        _ => "Verbindung fehlgeschlagen",
    };
    io::Error::new(
        io::ErrorKind::Other,
        format!("Netzlaufwerk {share}: {what} (WNet {code})"),
    )
}

pub(super) fn connect_impl(
    share: &str,
    user: Option<&str>,
    password: Option<&str>,
) -> io::Result<()> {
    use windows_sys::Win32::NetworkManagement::WNet::{
        WNetAddConnection2W, NETRESOURCEW, RESOURCETYPE_DISK,
    };
    let mut remote = to_wide(share);
    let user_w = user.map(to_wide);
    let pass_w = password.map(to_wide);

    let mut nr: NETRESOURCEW = unsafe { std::mem::zeroed() };
    nr.dwType = RESOURCETYPE_DISK;
    nr.lpRemoteName = remote.as_mut_ptr();

    let user_ptr = user_w.as_ref().map(|v| v.as_ptr()).unwrap_or(std::ptr::null());
    let pass_ptr = pass_w.as_ref().map(|v| v.as_ptr()).unwrap_or(std::ptr::null());

    // dwflags = 0 (no drive mapping, not persistent).
    let rc = unsafe { WNetAddConnection2W(&nr, pass_ptr, user_ptr, 0) };
    if rc == 0 {
        Ok(())
    } else {
        Err(wnet_error(rc, share))
    }
}

pub(super) fn disconnect_impl(share: &str) {
    use windows_sys::Win32::NetworkManagement::WNet::WNetCancelConnection2W;
    let name = to_wide(share);
    // force = FALSE (don't drop open handles abruptly).
    unsafe {
        let _ = WNetCancelConnection2W(name.as_ptr(), 0, 0);
    }
}
