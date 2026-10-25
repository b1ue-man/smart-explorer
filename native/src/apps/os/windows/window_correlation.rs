//! Capture provenance while the OS-owned process handle is still open.
use windows_sys::Win32::{Foundation::{FILETIME,HANDLE,CloseHandle,DuplicateHandle,DUPLICATE_SAME_ACCESS,WAIT_TIMEOUT},System::{Threading::{GetProcessId,GetProcessTimes,GetCurrentProcess,WaitForSingleObject},RemoteDesktop::ProcessIdToSessionId}};
use crate::apps::{LaunchRequest,HostSession};

pub(super) fn record(process:windows::Win32::Foundation::HANDLE,request:&LaunchRequest,expected:&HostSession,dispatch:&str) {
    struct Process(usize);
    impl Drop for Process {fn drop(&mut self) {unsafe {CloseHandle(self.0 as HANDLE);}}}
    let result=(||->std::io::Result<(String,std::sync::Arc<Process>)>{
        if !super::session::matches_process(process.0,expected)? {return Err(std::io::Error::other("started_process_user_or_logon_mismatch"));}
        let pid=unsafe {GetProcessId(process.0)};let mut session=0;
        if pid==0||unsafe {ProcessIdToSessionId(pid,&mut session)}==0 {return Err(std::io::Error::last_os_error());}
        let mut created:FILETIME=unsafe {std::mem::zeroed()};let mut exited=created;let mut kernel=created;let mut user=created;
        if unsafe {GetProcessTimes(process.0,&mut created,&mut exited,&mut kernel,&mut user)}==0 {return Err(std::io::Error::last_os_error());}
        let birth=(u64::from(created.dwHighDateTime)<<32)|u64::from(created.dwLowDateTime);
        let mut duplicate=std::ptr::null_mut();
        if unsafe {DuplicateHandle(GetCurrentProcess(),process.0,GetCurrentProcess(),&mut duplicate,0,0,DUPLICATE_SAME_ACCESS)}==0 {return Err(std::io::Error::last_os_error());}
        Ok((format!("{session}:{pid}:{birth}"),std::sync::Arc::new(Process(duplicate as usize))))
    })();
    if let Ok((key,process))=result {super::super::shared::window_correlation::record(request,dispatch,key,
        std::sync::Arc::new(move||unsafe {WaitForSingleObject(process.0 as HANDLE,0)==WAIT_TIMEOUT}));}
}
