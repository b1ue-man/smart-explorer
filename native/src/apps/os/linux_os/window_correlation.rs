//! The child sends its own kernel pidfd before exec. Identity stays exact even
//! with SIGCHLD handlers, external reapers and same-tick numeric PID reuse.
use std::{fs,io,process::{Child,Command},os::{fd::{OwnedFd,FromRawFd,AsRawFd},unix::{process::CommandExt,fs::MetadataExt}},sync::Arc};
use crate::apps::{LaunchRequest,HostSession};

pub(super) struct ChildWitness(OwnedFd);
pub(super) fn prepare(command:&mut Command)->io::Result<ChildWitness> {
    let mut pair=[-1;2];
    if unsafe {libc::socketpair(libc::AF_UNIX,libc::SOCK_DGRAM|libc::SOCK_CLOEXEC|libc::SOCK_NONBLOCK,0,pair.as_mut_ptr())}!=0 {return Err(io::Error::last_os_error());}
    let parent=unsafe {OwnedFd::from_raw_fd(pair[0])};let child=Arc::new(unsafe {OwnedFd::from_raw_fd(pair[1])});
    // SAFETY: the child callback uses only fixed stack storage and raw syscalls;
    // it never allocates, locks, logs, changes argv/env, or fails the app start.
    unsafe {command.pre_exec(move|| {send_self(child.as_raw_fd());Ok(())});}
    Ok(ChildWitness(parent))
}
fn send_self(socket:i32) {
    unsafe {
        let fd=libc::syscall(libc::SYS_pidfd_open,libc::getpid(),0) as i32;
        if fd<0 {return;}
        let mut data=[b'P'];let mut iov=libc::iovec {iov_base:data.as_mut_ptr().cast(),iov_len:1};
        let mut control=[0usize;8];let mut message:libc::msghdr=std::mem::zeroed();
        message.msg_iov=&mut iov;message.msg_iovlen=1;message.msg_control=control.as_mut_ptr().cast();
        message.msg_controllen=libc::CMSG_SPACE(std::mem::size_of::<i32>() as u32) as usize;
        let header=libc::CMSG_FIRSTHDR(&message);
        if !header.is_null() {
            (*header).cmsg_level=libc::SOL_SOCKET;(*header).cmsg_type=libc::SCM_RIGHTS;
            (*header).cmsg_len=libc::CMSG_LEN(std::mem::size_of::<i32>() as u32) as usize;
            libc::CMSG_DATA(header).cast::<i32>().write(fd);
            let _=libc::sendmsg(socket,&message,libc::MSG_DONTWAIT|libc::MSG_NOSIGNAL);
        }
        libc::close(fd);
    }
}
impl ChildWitness {
    fn process(&self)->io::Result<Arc<OwnedFd>> {
        unsafe {
            let mut data=[0u8];let mut iov=libc::iovec {iov_base:data.as_mut_ptr().cast(),iov_len:1};
            let mut control=[0usize;8];let mut message:libc::msghdr=std::mem::zeroed();
            message.msg_iov=&mut iov;message.msg_iovlen=1;message.msg_control=control.as_mut_ptr().cast();message.msg_controllen=std::mem::size_of_val(&control);
            if libc::recvmsg(self.0.as_raw_fd(),&mut message,libc::MSG_DONTWAIT|libc::MSG_CMSG_CLOEXEC)!=1 {return Err(io::Error::last_os_error());}
            let header=libc::CMSG_FIRSTHDR(&message);
            if header.is_null()||(*header).cmsg_level!=libc::SOL_SOCKET||(*header).cmsg_type!=libc::SCM_RIGHTS
                ||(*header).cmsg_len!=libc::CMSG_LEN(std::mem::size_of::<i32>() as u32) as usize {
                return Err(io::Error::other("original_child_pidfd_unavailable"));
            }
            let fd=OwnedFd::from_raw_fd(libc::CMSG_DATA(header).cast::<i32>().read());
            if data!=[b'P']||message.msg_flags&libc::MSG_CTRUNC!=0 {return Err(io::Error::other("invalid_original_child_pidfd"));}
            Ok(Arc::new(fd))
        }
    }
}
fn alive(process:&OwnedFd)->bool {
    let mut fd=libc::pollfd {fd:process.as_raw_fd(),events:libc::POLLIN,revents:0};
    unsafe {libc::poll(&mut fd,1,0)==0}
}
fn image(pid:u32)->io::Result<(u64,u64)> {
    let executable=fs::metadata(format!("/proc/{pid}/exe"))?;Ok((executable.dev(),executable.ino()))
}
pub(super) fn record(witness:Option<&ChildWitness>,child:&mut Child,request:&LaunchRequest,expected:&HostSession,dispatch:&str) {
    let result=(||->io::Result<(String,Arc<OwnedFd>,(u64,u64))>{
        let pid=child.id();
        let process=witness.ok_or_else(||io::Error::other("original_child_pidfd_unavailable"))?.process()?;
        if !alive(&process) {return Err(io::Error::other("original_child_already_exited"));}
        let executable=image(pid)?;
        let status=fs::read_to_string(format!("/proc/{pid}/status"))?;
        let uid=status.lines().find_map(|l|l.strip_prefix("Uid:").and_then(|v|v.split_whitespace().next()).and_then(|v|v.parse::<u32>().ok()))
            .ok_or_else(||io::Error::other("started_process_uid_unavailable"))?;
        if request.user_id.as_str()!=format!("linux-uid:{uid}") {return Err(io::Error::other("started_process_user_mismatch"));}
        let stat=fs::read_to_string(format!("/proc/{pid}/stat"))?;
        let values:Vec<_>=stat.rsplit_once(')').ok_or_else(||io::Error::other("invalid_started_process_stat"))?.1.split_whitespace().collect();
        let session=values.get(3).and_then(|s|s.parse::<u32>().ok()).ok_or_else(||io::Error::other("started_process_session_unavailable"))?;
        let start=values.get(19).and_then(|s|s.parse::<u64>().ok()).ok_or_else(||io::Error::other("started_process_incarnation_unavailable"))?;
        // The received kernel pidfd is the authority; wait status only rejects
        // an exited/reaped child. A numeric PID lookup never replaces the fd.
        if child.try_wait()?.is_some() {return Err(io::Error::other("started_process_already_exited"));}
        let current=super::session::current()?;
        if current.user_id!=expected.user_id||current.session_id!=expected.session_id {return Err(io::Error::other("app_logon_changed_after_spawn"));}
        if !alive(&process)||image(pid)?!=executable {return Err(io::Error::other("original_child_exited_or_exec_changed_during_identity_read"));}
        Ok((format!("x11:{uid}:{session}:{pid}:{start}"),process,executable))
    })();
    let pid=child.id();
    if let Ok((key,process,executable))=result {super::super::shared::window_correlation::record(request,dispatch,key,Arc::new(move||
        alive(&process)&&image(pid).is_ok_and(|current|current==executable)&&alive(&process)));}
}
