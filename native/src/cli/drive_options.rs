//! Mount creation options, shared policy defaults and typed CLI validation.

use clap::Args;

#[derive(Args)]
pub(super) struct MountArgs {
    #[arg(help = "Saved @label:/path, remote URL, gdrive:// path, or share:// endpoint")]
    pub(super) target: String,
    #[arg(
        long,
        default_value = "auto",
        value_name = "AUTO|LETTER",
        help = "Choose a drive letter automatically or request one letter"
    )]
    pub(super) letter: String,
    #[arg(long, help = "Enable writes; the default is read-only and safer for remote protocols")]
    pub(super) read_write: bool,
    #[arg(
        long,
        help = "Trust the selected root when the active backend cannot provide race-proof root confinement"
    )]
    pub(super) trust_remote_root: bool,
    #[arg(
        long,
        default_value_t = crate::mount::DEFAULT_METADATA_PRELOAD_DEPTH,
        value_parser = clap::value_parser!(u8).range(0..=4),
        help = "Preload directory metadata to this depth; 0 caches only opened folders"
    )]
    pub(super) metadata_depth: u8,
    #[arg(
        long,
        default_value_t = crate::mount::DEFAULT_MOUNT_CACHE_MIB,
        value_parser = clap::value_parser!(u32).range(0..=i64::from(crate::mount::MAX_MOUNT_CACHE_MIB)),
        value_name = "MIB",
        help = "Retain up to this many MiB of idle clean files; 0 disables retention. Open/unsaved data is excluded"
    )]
    pub(super) cache_mib: u32,
    #[arg(
        long,
        help = "Use the official System32 Dokany DLL for compatibility instead of the optimized private runtime"
    )]
    pub(super) system_runtime: bool,
    #[arg(long, value_name = "TEXT", help = "Windows volume label")]
    pub(super) label: Option<String>,
    #[arg(long, help = "Print machine-readable JSON")]
    pub(super) json: bool,
}
