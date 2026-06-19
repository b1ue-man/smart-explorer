#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Protocol {
    Sftp,
    Ftp,
    Ftps,
    Webdav,
    Share,
}

impl Protocol {
    pub fn as_str(self) -> &'static str {
        match self {
            Protocol::Sftp => "sftp",
            Protocol::Ftp => "ftp",
            Protocol::Ftps => "ftps",
            Protocol::Webdav => "webdav",
            Protocol::Share => "share",
        }
    }

    pub fn parse(s: &str) -> Option<Protocol> {
        match s {
            "sftp" => Some(Protocol::Sftp),
            "ftp" => Some(Protocol::Ftp),
            "ftps" => Some(Protocol::Ftps),
            "webdav" => Some(Protocol::Webdav),
            "share" => Some(Protocol::Share),
            _ => None,
        }
    }

    pub fn default_port(self) -> u16 {
        match self {
            Protocol::Sftp => 22,
            Protocol::Ftp | Protocol::Ftps => 21,
            Protocol::Webdav => 443,
            Protocol::Share => 0,
        }
    }

    pub fn is_url(self) -> bool {
        !matches!(self, Protocol::Share)
    }
}

#[derive(Clone, PartialEq, Eq, Debug)]
pub enum AuthKind {
    Password,
    Key { path: String },
}

#[derive(Clone, Debug)]
pub struct SavedConnection {
    pub protocol: Protocol,
    pub host: String,
    pub port: u16,
    pub user: String,
    pub auth: AuthKind,
    /// Remote start path (sftp/ftp) or the `\\server\share…` UNC (share).
    pub root: String,
    pub label: String,
    /// Opt-in: deploy + use the SSH remote agent for this connection (#24).
    /// SFTP only; ignored elsewhere. Defaults false (old entries → false).
    pub use_agent: bool,
}

impl SavedConnection {
    /// Stable, unique keyring account key for this connection's secret.
    pub fn account(&self) -> String {
        format!(
            "{}://{}@{}:{}{}",
            self.protocol.as_str(),
            self.user,
            self.host,
            self.port,
            self.root
        )
    }

    /// Navigation target: a `proto://user@host:port/root` URL for sftp/ftp/ftps,
    /// or the UNC root for a share.
    pub fn to_target(&self) -> String {
        if self.protocol.is_url() {
            format!(
                "{}://{}@{}:{}{}",
                self.protocol.as_str(),
                self.user,
                self.host,
                self.port,
                self.root
            )
        } else {
            self.root.clone()
        }
    }

    pub fn display(&self) -> String {
        if self.label.trim().is_empty() {
            self.account()
        } else {
            self.label.clone()
        }
    }
}

fn sanitize(s: &str) -> String {
    s.replace(['\t', '\n', '\r'], " ")
}

pub(super) fn serialize(c: &SavedConnection) -> String {
    let (auth, keypath) = match &c.auth {
        AuthKind::Password => ("password", String::new()),
        AuthKind::Key { path } => ("key", path.clone()),
    };
    [
        c.protocol.as_str().to_string(),
        sanitize(&c.host),
        c.port.to_string(),
        sanitize(&c.user),
        auth.to_string(),
        sanitize(&keypath),
        sanitize(&c.root),
        sanitize(&c.label),
        if c.use_agent { "1" } else { "0" }.to_string(),
    ]
    .join("\t")
}

pub(super) fn parse(line: &str) -> Option<SavedConnection> {
    let f: Vec<&str> = line.split('\t').collect();
    if f.len() < 8 {
        return None;
    }
    let protocol = Protocol::parse(f[0])?;
    let port = f[2].parse::<u16>().ok()?;
    let auth = match f[4] {
        "key" => AuthKind::Key {
            path: f[5].to_string(),
        },
        _ => AuthKind::Password,
    };
    Some(SavedConnection {
        protocol,
        host: f[1].to_string(),
        port,
        user: f[3].to_string(),
        auth,
        root: f[6].to_string(),
        label: f[7].to_string(),
        // Field 9 added in #24; old 8-field lines default to false.
        use_agent: f.get(8).map(|v| *v == "1").unwrap_or(false),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_pw() -> SavedConnection {
        SavedConnection {
            protocol: Protocol::Sftp,
            host: "example.com".into(),
            port: 2222,
            user: "alice".into(),
            auth: AuthKind::Password,
            root: "/home/alice".into(),
            label: "Work box".into(),
            use_agent: false,
        }
    }

    #[test]
    fn serialize_parse_roundtrip_password() {
        let c = sample_pw();
        let line = serialize(&c);
        let back = parse(&line).unwrap();
        assert_eq!(back.protocol, Protocol::Sftp);
        assert_eq!(back.host, "example.com");
        assert_eq!(back.port, 2222);
        assert_eq!(back.user, "alice");
        assert_eq!(back.auth, AuthKind::Password);
        assert_eq!(back.root, "/home/alice");
        assert_eq!(back.label, "Work box");
    }

    #[test]
    fn serialize_parse_roundtrip_key() {
        let mut c = sample_pw();
        c.auth = AuthKind::Key {
            path: "C:/keys/id_ed25519".into(),
        };
        c.protocol = Protocol::Ftps;
        let back = parse(&serialize(&c)).unwrap();
        assert_eq!(back.protocol, Protocol::Ftps);
        assert_eq!(
            back.auth,
            AuthKind::Key {
                path: "C:/keys/id_ed25519".into()
            }
        );
    }

    #[test]
    fn account_and_target_formats() {
        let c = sample_pw();
        assert_eq!(c.account(), "sftp://alice@example.com:2222/home/alice");
        assert_eq!(c.to_target(), "sftp://alice@example.com:2222/home/alice");

        let share = SavedConnection {
            protocol: Protocol::Share,
            host: "fileserver".into(),
            port: 0,
            user: "dom\\bob".into(),
            auth: AuthKind::Password,
            root: r"\\fileserver\public".into(),
            label: String::new(),
            use_agent: false,
        };
        assert_eq!(share.to_target(), r"\\fileserver\public");
        assert!(!share.protocol.is_url());
    }
}
