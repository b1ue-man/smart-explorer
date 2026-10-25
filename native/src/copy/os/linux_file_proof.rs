//! Restart-stable identity on filesystems with a real exported incarnation.
//! Shared verbatim with se-agent; this file has no application dependencies.
use std::{fs::File, io, os::unix::{fs::MetadataExt, io::AsRawFd}};

/// This is deliberately independent of st_dev/st_ino, file size and clocks.
/// The UUID comes from this handle's superblock, and the opaque export handle
/// includes the filesystem's inode generation (and Btrfs subvolume identity).
/// Missing kernel/filesystem support is a capability refusal, never a weaker ID.
pub(crate) fn stable_handle_identity(file: &File) -> io::Result<String> {
    #[cfg(any(target_os = "linux", target_os = "android"))]
    {
        let metadata = file.metadata()?;
        if !metadata.is_file() {
            return Err(io::Error::new(io::ErrorKind::InvalidInput, "file identity requires a regular file"));
        }
        let mut facts = std::mem::MaybeUninit::<libc::statfs>::uninit();
        // SAFETY: the live descriptor and correctly sized output remain valid.
        if unsafe { libc::fstatfs(file.as_raw_fd(), facts.as_mut_ptr()) } != 0 {
            return Err(io::Error::last_os_error());
        }
        let kind = unsafe { facts.assume_init() }.f_type as u64 & 0xffff_ffff;
        // Only these reviewed export encoders carry a real inode generation.
        // In particular FAT, FUSE, overlay and arbitrary network IDs do not
        // acquire a persistent ownership promise through this API.
        if !matches!(kind, 0xef53 | 0x5846_5342 | 0x9123_683e) {
            return Err(unsupported("filesystem has no qualified persistent file incarnation"));
        }
        let query = ioctl_handle(file)?;
        let uuid = filesystem_uuid(&query)?;
        let handle = export_handle(file)?;
        qualify_export(kind, &handle)?;
        Ok(format!("linux-file-v2:{kind:x}:{}:{}:{}", hex(&uuid), handle.kind,
            hex(&handle.bytes[..handle.length as usize])))
    }
    #[cfg(not(any(target_os = "linux", target_os = "android")))]
    {
        let _ = file;
        Err(unsupported("OS has no qualified persistent file incarnation"))
    }
}

#[cfg(any(target_os = "linux", target_os = "android"))]
fn ioctl_handle(file: &File) -> io::Result<File> {
    use std::os::unix::fs::OpenOptionsExt;
    // O_PATH supports metadata/export-handle queries, but not ioctl. Reopen
    // only that already retained object through procfs, never a mutable name.
    let flags = unsafe { libc::fcntl(file.as_raw_fd(), libc::F_GETFL) };
    if flags < 0 { return Err(io::Error::last_os_error()); }
    if flags & libc::O_PATH == 0 { return file.try_clone(); }
    let reopened = std::fs::OpenOptions::new().read(true)
        .custom_flags(libc::O_NONBLOCK | libc::O_NOCTTY)
        .open(format!("/proc/self/fd/{}", file.as_raw_fd()))?;
    let before = file.metadata()?;
    let after = reopened.metadata()?;
    // Both handles stay alive, so an inode cannot be freed/reused between them.
    if !after.is_file() || before.dev() != after.dev() || before.ino() != after.ino() {
        return Err(io::Error::new(io::ErrorKind::InvalidData, "held file changed during UUID query"));
    }
    Ok(reopened)
}

#[cfg(any(target_os = "linux", target_os = "android"))]
fn filesystem_uuid(file: &File) -> io::Result<[u8; 16]> {
    // Linux UAPI fs.h: FS_IOC_GETFSUUID = _IOR(0x15, 0, struct fsuuid2).
    // These supported architectures use the asm-generic ioctl bit layout.
    #[cfg(any(target_arch = "x86", target_arch = "x86_64", target_arch = "arm",
        target_arch = "aarch64", target_arch = "riscv32", target_arch = "riscv64"))]
    {
        #[repr(C)]
        struct FsUuid { length: u8, bytes: [u8; 16] }
        let mut uuid = FsUuid { length: 0, bytes: [0; 16] };
        // SAFETY: FS_IOC_GETFSUUID writes exactly this 17-byte UAPI structure.
        if unsafe { libc::ioctl(file.as_raw_fd(), 0x8011_1500u32 as _, &mut uuid as *mut FsUuid) } != 0 {
            return Err(capability_error(io::Error::last_os_error()));
        }
        if uuid.length != 16 || uuid.bytes == [0; 16] {
            return Err(unsupported("filesystem does not expose a full persistent UUID"));
        }
        Ok(uuid.bytes)
    }
    #[cfg(not(any(target_arch = "x86", target_arch = "x86_64", target_arch = "arm",
        target_arch = "aarch64", target_arch = "riscv32", target_arch = "riscv64")))]
    {
        let _ = file;
        Err(unsupported("persistent UUID ioctl ABI is not qualified on this architecture"))
    }
}

#[cfg(any(target_os = "linux", target_os = "android"))]
#[repr(C)]
struct ExportHandle { length: u32, kind: i32, bytes: [u8; 128] }

#[cfg(any(target_os = "linux", target_os = "android"))]
fn qualify_export(filesystem: u64, handle: &ExportHandle) -> io::Result<()> {
    let length = handle.length as usize;
    let generation = match (filesystem, handle.kind, length) {
        // generic_encode_ino32_fh, and XFS's 32/64-bit export formats.
        (0xef53, 1, 8) | (0x5846_5342, 1, 8) => &handle.bytes[4..8],
        (0x5846_5342, 0x81, 12) => &handle.bytes[8..12],
        // Packed btrfs_fid: objectid, root_objectid, gen; no parent requested.
        (0x9123_683e, 0x4d, 20) => &handle.bytes[16..20],
        _ => return Err(unsupported("filesystem export format has no qualified incarnation")),
    };
    // ext's and Btrfs's decoders treat zero as a wildcard; it is not an
    // incarnation. XFS documents zero as a real generation and checks it.
    if filesystem != 0x5846_5342 && generation == [0; 4] {
        return Err(unsupported("filesystem exported a wildcard inode generation"));
    }
    Ok(())
}

#[cfg(any(target_os = "linux", target_os = "android"))]
fn export_handle(file: &File) -> io::Result<ExportHandle> {
    let mut handle = ExportHandle { length: 128, kind: 0, bytes: [0; 128] };
    let mut mount = 0i32;
    // SAFETY: AT_EMPTY_PATH addresses this descriptor itself. The UAPI header
    // and its 128-byte payload have their declared C layout and stay writable.
    // No AT_HANDLE_FID fallback: a merely non-decodable inode ID is insufficient.
    let result = unsafe { libc::syscall(libc::SYS_name_to_handle_at,
        file.as_raw_fd(), b"\0".as_ptr().cast::<libc::c_char>(),
        &mut handle as *mut ExportHandle, &mut mount as *mut i32, libc::AT_EMPTY_PATH) };
    if result != 0 { return Err(capability_error(io::Error::last_os_error())); }
    if handle.length == 0 || handle.length > 128 || handle.kind <= 0 || handle.kind == 255 {
        return Err(unsupported("filesystem returned no usable exported file incarnation"));
    }
    Ok(handle)
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

fn unsupported(message: &str) -> io::Error {
    io::Error::new(io::ErrorKind::Unsupported, message)
}

#[cfg(any(target_os = "linux", target_os = "android"))]
fn capability_error(error: io::Error) -> io::Error {
    if matches!(error.raw_os_error(), Some(libc::ENOTTY | libc::ENOSYS | libc::EINVAL
        | libc::EOPNOTSUPP | libc::EOVERFLOW | libc::EPERM | libc::EACCES)) {
        io::Error::new(io::ErrorKind::Unsupported, error)
    } else { error }
}
