use std::ffi::CString;
use std::io;
use std::os::fd::{FromRawFd, OwnedFd};
use std::os::unix::ffi::OsStrExt;
use std::path::Path;

const CREATE_RULESET_VERSION: u32 = 1;
const RULE_PATH_BENEATH: u32 = 1;
const RESOLVE_NO_MAGICLINKS: u64 = 0x02;
const RESOLVE_NO_SYMLINKS: u64 = 0x04;

const FS_EXECUTE: u64 = 1 << 0;
const FS_WRITE_FILE: u64 = 1 << 1;
const FS_READ_FILE: u64 = 1 << 2;
const FS_READ_DIR: u64 = 1 << 3;
const FS_REMOVE_DIR: u64 = 1 << 4;
const FS_REMOVE_FILE: u64 = 1 << 5;
const FS_MAKE_CHAR: u64 = 1 << 6;
const FS_MAKE_DIR: u64 = 1 << 7;
const FS_MAKE_REG: u64 = 1 << 8;
const FS_MAKE_SOCK: u64 = 1 << 9;
const FS_MAKE_FIFO: u64 = 1 << 10;
const FS_MAKE_BLOCK: u64 = 1 << 11;
const FS_MAKE_SYM: u64 = 1 << 12;
const FS_REFER: u64 = 1 << 13;
const FS_TRUNCATE: u64 = 1 << 14;
const FS_IOCTL_DEV: u64 = 1 << 15;

const ABI_ONE_RIGHTS: u64 = FS_EXECUTE
    | FS_WRITE_FILE
    | FS_READ_FILE
    | FS_READ_DIR
    | FS_REMOVE_DIR
    | FS_REMOVE_FILE
    | FS_MAKE_CHAR
    | FS_MAKE_DIR
    | FS_MAKE_REG
    | FS_MAKE_SOCK
    | FS_MAKE_FIFO
    | FS_MAKE_BLOCK
    | FS_MAKE_SYM;

#[repr(C)]
struct RulesetAttr {
    handled_access_fs: u64,
}

#[repr(C)]
struct PathBeneathAttr {
    allowed_access: u64,
    parent_fd: i32,
}

#[repr(C)]
struct OpenHow {
    flags: u64,
    mode: u64,
    resolve: u64,
}

/// Enter an unprivileged, kernel-enforced filesystem domain before the agent
/// starts any worker thread. The selected directory is the only hierarchy in
/// which protocol operations may read or mutate data. Landlock ABI 3 is the
/// minimum because ABI 2 cannot confine truncate, `O_TRUNC`, or `ftruncate`.
pub fn restrict_filesystem(root: &Path) -> io::Result<()> {
    let abi = syscall_result(unsafe {
        libc::syscall(
            libc::SYS_landlock_create_ruleset,
            std::ptr::null::<RulesetAttr>(),
            0usize,
            CREATE_RULESET_VERSION,
        )
    })?;
    if abi < 3 {
        return Err(io::Error::new(
            io::ErrorKind::Unsupported,
            "kernel has no Landlock ABI 3 root confinement",
        ));
    }

    let root = CString::new(root.as_os_str().as_bytes())
        .map_err(|_| io::Error::new(io::ErrorKind::InvalidInput, "agent root contains NUL"))?;
    let how = OpenHow {
        flags: (libc::O_PATH | libc::O_DIRECTORY | libc::O_CLOEXEC) as u64,
        mode: 0,
        resolve: RESOLVE_NO_SYMLINKS | RESOLVE_NO_MAGICLINKS,
    };
    let root_fd = syscall_fd(unsafe {
        libc::syscall(
            libc::SYS_openat2,
            libc::AT_FDCWD,
            root.as_ptr(),
            &how,
            std::mem::size_of::<OpenHow>(),
        )
    })?;

    let mut handled = ABI_ONE_RIGHTS | FS_REFER;
    handled |= FS_TRUNCATE;
    if abi >= 5 {
        handled |= FS_IOCTL_DEV;
    }
    let ruleset_attr = RulesetAttr {
        handled_access_fs: handled,
    };
    let ruleset_fd = syscall_fd(unsafe {
        libc::syscall(
            libc::SYS_landlock_create_ruleset,
            &ruleset_attr,
            std::mem::size_of::<RulesetAttr>(),
            0u32,
        )
    })?;

    let mut allowed = FS_WRITE_FILE
        | FS_READ_FILE
        | FS_READ_DIR
        | FS_REMOVE_DIR
        | FS_REMOVE_FILE
        | FS_MAKE_DIR
        | FS_MAKE_REG
        | FS_REFER;
    allowed |= FS_TRUNCATE;
    let path_rule = PathBeneathAttr {
        allowed_access: allowed,
        parent_fd: raw_fd(&root_fd),
    };
    syscall_result(unsafe {
        libc::syscall(
            libc::SYS_landlock_add_rule,
            raw_fd(&ruleset_fd),
            RULE_PATH_BENEATH,
            &path_rule,
            0u32,
        )
    })?;

    if unsafe { libc::prctl(libc::PR_SET_NO_NEW_PRIVS, 1, 0, 0, 0) } != 0 {
        return Err(io::Error::last_os_error());
    }
    syscall_result(unsafe {
        libc::syscall(libc::SYS_landlock_restrict_self, raw_fd(&ruleset_fd), 0u32)
    })?;
    Ok(())
}

fn raw_fd(fd: &OwnedFd) -> i32 {
    std::os::fd::AsRawFd::as_raw_fd(fd)
}

fn syscall_fd(result: libc::c_long) -> io::Result<OwnedFd> {
    let raw = syscall_result(result)?;
    let raw = i32::try_from(raw)
        .map_err(|_| io::Error::other("Landlock returned an invalid descriptor"))?;
    // SAFETY: a successful open/create_ruleset syscall returns one newly owned
    // descriptor, transferred directly into this RAII owner.
    Ok(unsafe { OwnedFd::from_raw_fd(raw) })
}

fn syscall_result(result: libc::c_long) -> io::Result<libc::c_long> {
    if result < 0 {
        Err(io::Error::last_os_error())
    } else {
        Ok(result)
    }
}
