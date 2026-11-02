use std::ffi::OsStr;
use std::fs::File;
use std::io;
use std::mem::{size_of, size_of_val, zeroed};
use std::net::SocketAddr;
use std::os::windows::ffi::OsStrExt;
use std::os::windows::io::FromRawHandle;
use std::path::Path;

use windows_sys::Win32::Foundation::{
    SetHandleInformation, GENERIC_READ, GENERIC_WRITE, HANDLE, HANDLE_FLAG_INHERIT,
};
use windows_sys::Win32::Security::SECURITY_ATTRIBUTES;
use windows_sys::Win32::Storage::FileSystem::{
    CreateFileW, FILE_ATTRIBUTE_NORMAL, FILE_SHARE_READ, FILE_SHARE_WRITE, OPEN_EXISTING,
};
use windows_sys::Win32::System::Pipes::CreatePipe;
use windows_sys::Win32::System::Threading::{
    CreateProcessW, DeleteProcThreadAttributeList, InitializeProcThreadAttributeList, ResumeThread,
    UpdateProcThreadAttribute, CREATE_NO_WINDOW, CREATE_SUSPENDED, CREATE_UNICODE_ENVIRONMENT,
    EXTENDED_STARTUPINFO_PRESENT, PROCESS_INFORMATION, PROC_THREAD_ATTRIBUTE_HANDLE_LIST,
    PROC_THREAD_ATTRIBUTE_JOB_LIST, STARTF_USESTDHANDLES, STARTUPINFOEXW,
};

use super::mount_job::{MountHostChild, MountHostJob, OwnedHandle};
use super::mount_process_environment;

pub(super) struct LaunchedMountHost {
    pub(super) child: MountHostChild,
    pub(super) stderr: File,
    pub(super) job: MountHostJob,
}

pub(super) fn launch(
    mount_id: &crate::mount::MountId,
    system_windows_directory: &OsStr,
    launch_token: &str,
    ipc_addr: SocketAddr,
    cache_root: &Path,
) -> io::Result<LaunchedMountHost> {
    let job = MountHostJob::create()?;
    let executable = wide_nul(std::env::current_exe()?.as_os_str())?;
    let mut command_line = format!("smart_explorer --mount-host {}", mount_id.as_str())
        .encode_utf16()
        .chain(Some(0))
        .collect::<Vec<_>>();
    let mut environment =
        environment_block(system_windows_directory, launch_token, ipc_addr, cache_root)?;
    let child_stdin = open_null(GENERIC_READ)?;
    let child_stdout = open_null(GENERIC_WRITE)?;
    let (child_stderr, parent_stderr) = stderr_pipe()?;
    let inherited = [child_stdin.raw(), child_stdout.raw(), child_stderr.raw()];
    let mut attributes = AttributeList::new(2)?;
    attributes.set(
        PROC_THREAD_ATTRIBUTE_HANDLE_LIST as usize,
        inherited.as_ptr().cast(),
        size_of_val(&inherited),
    )?;
    let job_handle = job.raw();
    attributes.set(
        PROC_THREAD_ATTRIBUTE_JOB_LIST as usize,
        (&job_handle as *const HANDLE).cast(),
        size_of::<HANDLE>(),
    )?;

    let mut startup: STARTUPINFOEXW = unsafe { zeroed() };
    startup.StartupInfo.cb = size_of::<STARTUPINFOEXW>() as u32;
    startup.StartupInfo.dwFlags = STARTF_USESTDHANDLES;
    startup.StartupInfo.hStdInput = child_stdin.raw();
    startup.StartupInfo.hStdOutput = child_stdout.raw();
    startup.StartupInfo.hStdError = child_stderr.raw();
    startup.lpAttributeList = attributes.ptr();
    let mut process: PROCESS_INFORMATION = unsafe { zeroed() };
    let created = unsafe {
        CreateProcessW(
            executable.as_ptr(),
            command_line.as_mut_ptr(),
            std::ptr::null(),
            std::ptr::null(),
            1,
            EXTENDED_STARTUPINFO_PRESENT
                | CREATE_UNICODE_ENVIRONMENT
                | CREATE_SUSPENDED
                | CREATE_NO_WINDOW,
            environment.as_mut_ptr().cast(),
            std::ptr::null(),
            &startup.StartupInfo,
            &mut process,
        )
    };
    if created == 0 {
        return Err(io::Error::last_os_error());
    }
    let process_handle = OwnedHandle::new(process.hProcess);
    let thread_handle = OwnedHandle::new(process.hThread);
    let process_handle = process_handle?;
    let thread_handle = thread_handle?;
    if unsafe { ResumeThread(thread_handle.raw()) } == u32::MAX {
        return Err(io::Error::last_os_error());
    }
    Ok(LaunchedMountHost {
        child: MountHostChild::from_owned(process_handle),
        stderr: parent_stderr,
        job,
    })
}

fn environment_block(
    system_windows_directory: &OsStr,
    launch_token: &str,
    ipc_addr: SocketAddr,
    cache_root: &Path,
) -> io::Result<Vec<u16>> {
    let mut values = mount_process_environment::values(
        system_windows_directory,
        launch_token,
        ipc_addr,
        cache_root,
    );
    values.sort_by(|left, right| {
        left.0
            .to_ascii_uppercase()
            .cmp(&right.0.to_ascii_uppercase())
    });
    let mut block = Vec::new();
    for (name, value) in values {
        block.extend(name.encode_utf16());
        block.push(b'=' as u16);
        let encoded = value.encode_wide().collect::<Vec<_>>();
        if encoded.contains(&0) {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "mount-host environment contains NUL",
            ));
        }
        block.extend(encoded);
        block.push(0);
    }
    block.push(0);
    Ok(block)
}

fn open_null(access: u32) -> io::Result<OwnedHandle> {
    let attributes = inheritable_attributes();
    let name = [b'N' as u16, b'U' as u16, b'L' as u16, 0];
    OwnedHandle::new(unsafe {
        CreateFileW(
            name.as_ptr(),
            access,
            FILE_SHARE_READ | FILE_SHARE_WRITE,
            &attributes,
            OPEN_EXISTING,
            FILE_ATTRIBUTE_NORMAL,
            std::ptr::null_mut(),
        )
    })
}

fn stderr_pipe() -> io::Result<(OwnedHandle, File)> {
    let attributes = inheritable_attributes();
    let mut read = std::ptr::null_mut();
    let mut write = std::ptr::null_mut();
    if unsafe { CreatePipe(&mut read, &mut write, &attributes, 0) } == 0 {
        return Err(io::Error::last_os_error());
    }
    let read = OwnedHandle::new(read);
    let write = OwnedHandle::new(write);
    let read = read?;
    let write = write?;
    if unsafe { SetHandleInformation(read.raw(), HANDLE_FLAG_INHERIT, 0) } == 0 {
        return Err(io::Error::last_os_error());
    }
    let parent = unsafe { File::from_raw_handle(read.into_raw().cast()) };
    Ok((write, parent))
}

fn inheritable_attributes() -> SECURITY_ATTRIBUTES {
    SECURITY_ATTRIBUTES {
        nLength: size_of::<SECURITY_ATTRIBUTES>() as u32,
        lpSecurityDescriptor: std::ptr::null_mut(),
        bInheritHandle: 1,
    }
}

fn wide_nul(value: &OsStr) -> io::Result<Vec<u16>> {
    let encoded = value.encode_wide().collect::<Vec<_>>();
    if encoded.contains(&0) {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "mount-host executable path contains NUL",
        ));
    }
    Ok(encoded.into_iter().chain(Some(0)).collect())
}

struct AttributeList {
    bytes: Vec<usize>,
    initialized: bool,
}

impl AttributeList {
    fn new(count: u32) -> io::Result<Self> {
        let mut size = 0usize;
        unsafe { InitializeProcThreadAttributeList(std::ptr::null_mut(), count, 0, &mut size) };
        if size == 0 {
            return Err(io::Error::last_os_error());
        }
        let mut list = Self {
            bytes: vec![0; size.div_ceil(size_of::<usize>())],
            initialized: false,
        };
        if unsafe { InitializeProcThreadAttributeList(list.ptr(), count, 0, &mut size) } == 0 {
            return Err(io::Error::last_os_error());
        }
        list.initialized = true;
        Ok(list)
    }

    fn ptr(&mut self) -> windows_sys::Win32::System::Threading::LPPROC_THREAD_ATTRIBUTE_LIST {
        self.bytes.as_mut_ptr().cast()
    }

    fn set(
        &mut self,
        attribute: usize,
        value: *const core::ffi::c_void,
        size: usize,
    ) -> io::Result<()> {
        if unsafe {
            UpdateProcThreadAttribute(
                self.ptr(),
                0,
                attribute,
                value,
                size,
                std::ptr::null_mut(),
                std::ptr::null(),
            )
        } == 0
        {
            Err(io::Error::last_os_error())
        } else {
            Ok(())
        }
    }
}

impl Drop for AttributeList {
    fn drop(&mut self) {
        if self.initialized {
            unsafe { DeleteProcThreadAttributeList(self.ptr()) };
        }
    }
}
