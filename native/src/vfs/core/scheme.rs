/// Which backend owns a path; independent of its host platform.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum Scheme {
    #[default]
    Local,
    Sftp,
    Ftp,
    Webdav,
    Smb,
    GDrive,
    Peer,
    /// The cleartext view of an unlocked Cryptomator vault (`vault://`),
    /// layered over the backend that stores the vault.
    Vault,
}
