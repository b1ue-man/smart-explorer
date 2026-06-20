use std::io;

pub(super) fn connect_impl(
    _share: &str,
    _user: Option<&str>,
    _password: Option<&str>,
) -> io::Result<()> {
    Err(io::Error::new(
        io::ErrorKind::Unsupported,
        "Netzlaufwerk-Authentifizierung nur unter Windows",
    ))
}

pub(super) fn disconnect_impl(_share: &str) {}
