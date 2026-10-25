use std::{io,mem::size_of};
use windows::{core::{w,PCWSTR},Win32::{Foundation::CloseHandle,System::Com::CoTaskMemFree,
    UI::{Shell::*,WindowsAndMessaging::SW_SHOWNORMAL}}};
use crate::apps::core::model::{HostSession,LaunchRequest,LaunchState};
use super::{catalog::{self,Target},session};
use super::super::shared::platform::{HostPlatform,PlatformCatalog,DispatchReceipt,expired};

#[derive(Default)]
pub(crate) struct Platform;
fn outcome(state:LaunchState,reason:impl Into<String>)->DispatchReceipt {DispatchReceipt {state,reason:reason.into(),pid:None}}
impl HostPlatform for Platform {
    type Target=Target;
    fn session(&self)->io::Result<HostSession> {session::current()}
    fn snapshot(&self)->io::Result<PlatformCatalog<Target>> {
        let session=session::current()?; let apps=catalog::list()?;
        if session::current()?!=session {return Err(io::Error::other("Windows logon changed while listing applications"));}
        Ok(PlatformCatalog {session,apps,limits:vec!["Application starts run only in this active user's interactive logon; an Exec service cannot launch into another session".into()]})
    }
    fn dispatch(&self,target:&Target,request:&LaunchRequest,expected:&HostSession,deadline:Option<i64>,dispatch_id:&str)->io::Result<DispatchReceipt> {
        let _apartment=catalog::Apartment::enter()?;
        let name:Vec<u16>=target.parsing_name.encode_utf16().chain(Some(0)).collect();
        let item:IShellItem=unsafe {SHCreateItemFromParsingName(PCWSTR(name.as_ptr()),None)}.map_err(catalog::error)?;
        if catalog::fingerprint(&item)?!=target.fingerprint {return Ok(outcome(LaunchState::Denied,"installed_shell_application_changed"));}
        // This check occurs after Shell binding, immediately before invocation.
        if !session::current().is_ok_and(|current|&current==expected) {return Ok(outcome(LaunchState::Denied,"interactive_logon_changed_or_unavailable"));}
        if !expected.unlocked {return Ok(outcome(LaunchState::ConsentRequired,"unlock_the_windows_host_session"));}
        let pidl=unsafe {SHGetIDListFromObject(&item)}.map_err(catalog::error)?;
        if expired(deadline) {
            unsafe {CoTaskMemFree(Some(pidl.cast()));}
            return Ok(outcome(LaunchState::Denied,"app_launch_grant_expired"));
        }
        let mut info=SHELLEXECUTEINFOW {cbSize:size_of::<SHELLEXECUTEINFOW>() as u32,
            fMask:SEE_MASK_IDLIST|SEE_MASK_NOCLOSEPROCESS|SEE_MASK_NOASYNC|SEE_MASK_FLAG_NO_UI,
            lpVerb:w!("open"),lpIDList:pidl.cast(),nShow:SW_SHOWNORMAL.0,..Default::default()};
        // Shell-owned PIDL, fixed verb, no lpFile/lpParameters from the peer.
        let result=unsafe {ShellExecuteExW(&mut info)};
        unsafe {CoTaskMemFree(Some(pidl.cast()));}
        if !info.hProcess.is_invalid() {
            if result.is_ok() {super::window_correlation::record(info.hProcess,request,expected,dispatch_id);}
            unsafe {let _=CloseHandle(info.hProcess);}
        }
        match result {
            Ok(())=>Ok(outcome(LaunchState::StartedWindowPending,"shell_start_accepted_window_not_yet_available")),
            Err(error)=>Ok(outcome(LaunchState::Denied,format!("shell_start_refused: {error}"))),
        }
    }
}
