//! x11rb 0.13.2 grows PacketReader immediately after byte 32. Withhold
//! that header until its exact raw/parsed owners have been admitted.
//! read never waits for Parent IPC; poll performs the admission instead.
use std::io::{self, IoSlice};
use std::os::fd::AsRawFd;
use std::sync::{Arc, Mutex, PoisonError};
use x11rb::connection::Connection;
use x11rb::rust_connection::{RustConnection, stream::{PollMode, Stream}};
use x11rb::utils::RawFdContainer;
use crate::screen::core::capture_admission::{CaptureAuthority, CaptureGrant, CaptureSlice};
#[cfg(test)]
use crate::screen::core::capture_admission::{CaptureGeneration, SourceKey};
use crate::screen::core::capture_plan::{CaptureOwner, CapturePhases, CapturePlan};

pub(crate) type CaptureConnection = RustConnection<AdmissionStream>;
enum Expected {
    Image {sequence: u16, bytes: u64, raw: CaptureSlice},
    Cursor {sequence: u16, image: CaptureGrant,
        authority: Arc<dyn CaptureAuthority>},
}
struct State {
    header: [u8; 32], filled: usize, sent: usize, body: usize,
    allowed: bool, expected: Option<Expected>,
    raw: Option<CaptureSlice>, parsed: Option<CaptureSlice>,
    // Failed native cleanup remains charged until the connection has died.
    retired: Vec<CaptureSlice>,
    failure: Option<String>,
}
impl Default for State {
    fn default() -> Self {Self {header: [0;32], filled:0, sent:0, body:0,
        allowed:false, expected:None, raw:None, parsed:None, retired:Vec::new(), failure:None}}
}
#[derive(Clone, Default)]
pub(crate) struct Gate(Arc<Mutex<State>>);
pub(crate) struct AdmissionStream {
    // This untouched RustConnection performs normal DISPLAY resolution,
    // authentication and setup. It never sends a post-setup request. The
    // outer connection starts at sequence one on that same transport.
    transport: RustConnection,
    gate: Gate,
}
fn invalid(text: &str) -> io::Error {io::Error::new(io::ErrorKind::InvalidData, text)}
fn blocked() -> io::Error {io::ErrorKind::WouldBlock.into()}
impl Gate {
    pub(crate) fn image(&self, sequence: u64, bytes: u64, grant: &CaptureGrant) -> io::Result<()> {
        let raw = grant.claim(CaptureOwner::Readback, bytes.checked_add(32).ok_or_else(||invalid("x11 reply overflow"))?.max(64))?;
        let mut state = self.0.lock().map_err(|_|invalid("x11 gate poisoned"))?;
        if state.expected.is_some() || state.raw.is_some() {return Err(invalid("x11 reply already armed"));}
        state.expected = Some(Expected::Image {sequence:sequence as u16, bytes, raw});
        Ok(())
    }
    pub(crate) fn cursor(&self, sequence: u64, grant: &CaptureGrant,
        authority: Arc<dyn CaptureAuthority>) -> io::Result<()> {
        let mut state = self.0.lock().map_err(|_|invalid("x11 gate poisoned"))?;
        if state.expected.is_some() || state.raw.is_some() {return Err(invalid("x11 reply already armed"));}
        state.expected = Some(Expected::Cursor {sequence:sequence as u16,
            image:grant.clone(), authority});
        Ok(())
    }
    /// Called only after Cookie::reply destroyed the raw PacketReader Vec.
    pub(crate) fn finish(&self) -> io::Result<Option<CaptureSlice>> {
        let mut state = self.0.lock().map_err(|_|invalid("x11 gate poisoned"))?;
        if let Some(failure) = &state.failure {return Err(invalid(failure));}
        state.expected.take(); state.raw.take();
        Ok(state.parsed.take())
    }
    pub(crate) fn retain(&self, slice: CaptureSlice) {
        self.0.lock().unwrap_or_else(PoisonError::into_inner).retired.push(slice);
    }
    fn authorize(&self) -> io::Result<bool> {
        let mut state = self.0.lock().map_err(|_|invalid("x11 gate poisoned"))?;
        if let Some(failure) = &state.failure {return Err(invalid(failure));}
        if state.filled != 32 || state.allowed {return Ok(state.allowed && state.sent < 32);}
        let header = state.header;
        let body = if header[0] == 1 || header[0] & 0x7f == 35 {
            u64::from(u32::from_ne_bytes(header[4..8].try_into().map_err(|_|invalid("x11 header"))?)) * 4
        } else {0};
        let sequence = u16::from_ne_bytes([header[2],header[3]]);
        let matching = state.expected.as_ref().is_some_and(|expected| match expected {
            Expected::Image {sequence: wanted,..} | Expected::Cursor {sequence: wanted,..} => *wanted == sequence,
        });
        if header[0] == 1 && matching {
            match state.expected.take().ok_or_else(||invalid("x11 missing expected reply"))? {
                Expected::Image {bytes, raw,..} => {
                    if body != bytes {state.retired.push(raw); return Err(invalid("x11 image reply length changed"));}
                    state.raw = Some(raw);
                }
                Expected::Cursor {image,authority,..} => {
                    let width = u16::from_ne_bytes([header[12],header[13]]);
                    let height = u16::from_ne_bytes([header[14],header[15]]);
                    let pixels = u64::from(width) * u64::from(height) * 4;
                    if body != pixels {return Err(invalid("x11 cursor geometry/reply mismatch"));}
                    if pixels > crate::screen::core::frame_budget::MAX_WORKING_BYTES
                        || usize::try_from(body + 32).ok().is_none_or(|bytes| bytes > isize::MAX as usize) {
                        return Err(invalid("x11 cursor reply exceeds raw allocation ceiling"));
                    }
                    let mut plan = CapturePlan::new(image.source().clone(),image.generation());
                    plan.add_bytes(CaptureOwner::Readback,(body+32).max(64),1,CapturePhases::ALL)?;
                    if pixels > 0 {plan.add_bytes(CaptureOwner::Metadata,pixels,1,CapturePhases::ALL)?;}
                    // poll, never read, is the only place that calls Parent.
                    let grant = authority.approve_supplement(&image,plan)?;
                    state.raw = Some(grant.claim(CaptureOwner::Readback,(body+32).max(64))?);
                    state.parsed = Some(grant.claim(CaptureOwner::Metadata,pixels)?);
                    // Both parser owners exist now; this one reply must not
                    // authorize later allocations. Its slices end separately.
                    grant.retire()?;
                }
            }
        } else if body > 64 * 1024 {
            return Err(invalid("unarmed large X11 reply"));
        }
        state.body = usize::try_from(body).map_err(|_|invalid("x11 reply exceeds address space"))?;
        state.allowed = true;
        Ok(true)
    }
}
impl AdmissionStream {
    pub(crate) fn gate(&self)->&Gate {&self.gate}

    pub(crate) fn connect() -> io::Result<(CaptureConnection, usize, Gate)> {
        let (transport, screen) = RustConnection::connect(None).map_err(io::Error::other)?;
        let setup = transport.setup().clone();
        let gate = Gate::default();
        let conn = RustConnection::for_connected_stream(Self {transport,gate:gate.clone()},setup)
            .map_err(io::Error::other)?;
        Ok((conn,screen,gate))
    }
}
impl Stream for AdmissionStream {
    fn poll(&self, mode: PollMode) -> io::Result<()> {
        if mode.readable() && self.gate.authorize().inspect_err(|error| {
            self.gate.0.lock().unwrap_or_else(PoisonError::into_inner).failure=Some(error.to_string());
        })? {return Ok(());}
        // Finite poll permits cancellation by closing this capture socket.
        let events = (if mode.readable() {libc::POLLIN} else {0})
            | (if mode.writable() {libc::POLLOUT} else {0});
        let mut fd = libc::pollfd {fd:self.transport.stream().as_raw_fd(),events,revents:0};
        loop {
            // SAFETY: one live socket descriptor; no pointer retained.
            let result = unsafe {libc::poll(&mut fd,1,250)};
            if result >= 0 {return Ok(());}
            let error = io::Error::last_os_error();
            if error.kind()!=io::ErrorKind::Interrupted {return Err(error);}
        }
    }
    fn read(&self, buf:&mut [u8], fds:&mut Vec<RawFdContainer>) -> io::Result<usize> {
        if buf.is_empty() {return Ok(0);}
        let mut state = self.gate.0.lock().map_err(|_|invalid("x11 gate poisoned"))?;
        if let Some(failure)=&state.failure {return Err(invalid(failure));}
        if state.filled < 32 {
            let filled = state.filled;
            let read = self.transport.stream().read(&mut state.header[filled..],fds)?;
            if read==0 {return Ok(0);}
            state.filled += read;
            return Err(blocked());
        }
        if !state.allowed {return Err(blocked());}
        if state.sent < 32 {
            let len = buf.len().min(32-state.sent);
            buf[..len].copy_from_slice(&state.header[state.sent..state.sent+len]);
            state.sent += len;
            if state.sent==32 && state.body==0 {state.filled=0;state.sent=0;state.allowed=false;}
            return Ok(len);
        }
        let len = buf.len().min(state.body);
        let read = self.transport.stream().read(&mut buf[..len],fds)?;
        state.body -= read;
        if state.body==0 {state.filled=0;state.sent=0;state.allowed=false;}
        Ok(read)
    }
    fn write(&self,buf:&[u8],fds:&mut Vec<RawFdContainer>)->io::Result<usize> {
        self.transport.stream().write(buf,fds)
    }
    fn write_vectored(&self,buf:&[IoSlice<'_>],fds:&mut Vec<RawFdContainer>)->io::Result<usize> {
        self.transport.stream().write_vectored(buf,fds)
    }
}

#[cfg(test)]
#[path="batches_capture_x11_task_tests.rs"]
mod tests;

#[cfg(test)]
mod supplement_tests {
    use super::*;
    #[test]
    fn batches_task_capture_linux_x11_supplement_grants_raw_and_argb_without_retiring_image() {
        let admission=crate::screen::core::capture_test_support::session(None);
        let source=SourceKey::new(admission.session(),"x11-cursor").unwrap();
        let mut plan=CapturePlan::new(source,CaptureGeneration::new(1));
        plan.add_bytes(CaptureOwner::SourcePixels,64,2,CapturePhases::ALL).unwrap();
        let image=admission.authority().approve(plan).unwrap();
        let gate=Gate::default();
        gate.cursor(7,&image,admission.authority()).unwrap();
        {
            let mut state=gate.0.lock().unwrap();
            state.header[0]=1;
            state.header[2..4].copy_from_slice(&7u16.to_ne_bytes());
            state.header[4..8].copy_from_slice(&8u32.to_ne_bytes());
            state.header[12..14].copy_from_slice(&4u16.to_ne_bytes());
            state.header[14..16].copy_from_slice(&2u16.to_ne_bytes());
            state.filled=32;
        }
        assert!(gate.authorize().unwrap());
        {
            let state=gate.0.lock().unwrap();
            assert_eq!(state.raw.as_ref().unwrap().bytes(),64);
            assert_eq!(state.parsed.as_ref().unwrap().bytes(),32);
        }
        let parsed=gate.finish().unwrap().unwrap();
        assert_eq!(parsed.grant().unwrap().live_bytes().unwrap(),32);
        assert!(!image.is_retired().unwrap());
        assert!(image.claim(CaptureOwner::SourcePixels,64).is_ok());
        drop(parsed);
        assert!(gate.0.lock().unwrap().failure.is_none());
    }
}
