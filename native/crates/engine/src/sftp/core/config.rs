use std::io;
use std::sync::Arc;
use zeroize::Zeroizing;

pub enum SftpAuth {
    Password(String),
    /// Private key file path + optional passphrase. Constructed by the Connect
    /// dialog (credential store) in the connect-UI step.
    #[allow(dead_code)]
    Key {
        path: String,
        passphrase: Option<String>,
    },
    /// The identities of the running SSH agent (`SSH_AUTH_SOCK`, the Windows
    /// OpenSSH agent or Pageant); no secret is stored.
    Agent,
}

pub struct SftpConfig {
    pub host: String,
    pub port: u16,
    pub user: String,
    pub auth: SftpAuth,
    /// Remote start directory (forward-slash, e.g. `/home/user`).
    pub root: String,
}

/// The sealed store's password for one login, read fresh each time (never
/// kept in plain text by the channel pool).
pub type PasswordSource = Arc<dyn Fn() -> io::Result<Zeroizing<String>> + Send + Sync>;

/// How the main login got its credentials (security answer E13-S15 (i)).
/// Extra SSH connections for transfers log in again without a person, so they
/// exist only when the main login needed no input either.
#[derive(Clone, Default)]
pub enum SftpLoginOrigin {
    /// A person typed something (password not saved, passphrase, one-time
    /// code), the origin is unknown, or the SSH agent signed: no extra
    /// connections. This is what `SftpBackend::connect` assumes.
    #[default]
    Interactive,
    /// `SftpAuth::Password`, taken from the sealed store without input; the
    /// source returns it again for every extra login.
    StoredPassword(PasswordSource),
    /// `SftpAuth::Key` whose passphrase (if any) came from the sealed store
    /// without input; the backend keeps the decrypted key of the main login.
    StoredKey,
}
