pub enum SftpAuth {
    Password(String),
    /// Private key file path + optional passphrase. Constructed by the Connect
    /// dialog (credential store) in the connect-UI step.
    #[allow(dead_code)]
    Key {
        path: String,
        passphrase: Option<String>,
    },
}

pub struct SftpConfig {
    pub host: String,
    pub port: u16,
    pub user: String,
    pub auth: SftpAuth,
    /// Remote start directory (forward-slash, e.g. `/home/user`).
    pub root: String,
}
