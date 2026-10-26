//! Opt-in operation errors. Internal debug checks must not consume the caller's evidence.
use std::{cell::Cell, marker::PhantomData, rc::Rc};
use glow::HasContext as _;

/// Errors observed during one checked operation, including internal macro reads.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct GlOperationError(u32);
impl GlOperationError {
    const OOM: u32 = 1 << 6;
    const LOST: u32 = 1 << 7;
    const UNKNOWN: u32 = 1 << 8;
    const CREATE: u32 = 1 << 9;
    const MISSING: u32 = 1 << 10;
    const CONFLICT: u32 = 1 << 11;
    const REPEATED: u32 = 1 << 12;
    pub fn from_gl_code(code: u32) -> Self {
        Self(match code {
            glow::NO_ERROR => 0,
            glow::INVALID_ENUM => 1,
            glow::INVALID_VALUE => 1 << 1,
            glow::INVALID_OPERATION => 1 << 2,
            glow::INVALID_FRAMEBUFFER_OPERATION => 1 << 3,
            glow::STACK_OVERFLOW => 1 << 4,
            glow::STACK_UNDERFLOW => 1 << 5,
            glow::OUT_OF_MEMORY => Self::OOM,
            glow::CONTEXT_LOST | 0x9242 => Self::LOST,
            _ => Self::UNKNOWN,
        })
    }
    pub fn texture_creation_failed() -> Self {Self(Self::CREATE)}
    pub(crate) fn missing_texture() -> Self {Self(Self::MISSING)}
    /// No claim of unchanged resource state is valid after these errors.
    pub fn state_unreliable(self) -> bool {
        self.0 & (Self::OOM | Self::LOST | Self::UNKNOWN | Self::CONFLICT | Self::REPEATED) != 0
    }
    pub fn contains_gl_error(self, code: u32) -> bool {
        let flag = Self::from_gl_code(code).0;
        flag != 0 && self.0 & flag != 0
    }
    fn any(self) -> bool {self.0 != 0}
}
impl std::fmt::Display for GlOperationError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "checked OpenGL operation failed (flags 0x{:X})", self.0)
    }
}
impl std::error::Error for GlOperationError {}

#[derive(Clone, Copy)]
struct Ledger {context: usize, depth: usize, error: GlOperationError}
impl Ledger {
    const EMPTY: Self = Self {context: 0, depth: 0, error: GlOperationError(0)};
}
thread_local! {static ERRORS: Cell<Ledger> = const {Cell::new(Ledger::EMPTY)};}
fn key(gl: &glow::Context) -> usize {gl as *const glow::Context as usize}
pub(crate) fn active(gl: &glow::Context) -> bool {active_key(key(gl))}
fn active_key(context: usize) -> bool {ERRORS.with(|cell| {let s=cell.get(); s.depth != 0 && s.context == context})}
fn record(context: usize, error: GlOperationError) {
    ERRORS.with(|cell| {
        let mut s=cell.get();
        if s.depth != 0 && s.context == context {s.error.0 |= error.0; cell.set(s);}
    });
}
struct ScopeGuard {context: usize, live: bool, _thread: PhantomData<Rc<()>>}
impl ScopeGuard {
    fn enter(context: usize) -> Result<Self, GlOperationError> {
        ERRORS.with(|cell| {
            let mut s=cell.get();
            if s.depth == 0 {s=Ledger {context, ..Ledger::EMPTY};}
            if s.context != context || s.depth == usize::MAX {
                s.error.0 |= GlOperationError::CONFLICT; cell.set(s);
                return Err(GlOperationError(GlOperationError::CONFLICT));
            }
            s.depth += 1; cell.set(s);
            Ok(Self {context, live: true, _thread: PhantomData})
        })
    }
    fn error(&self) -> GlOperationError {ERRORS.with(|cell|cell.get().error)}
    fn complete<T>(&mut self,result:Result<T,GlOperationError>)->Result<T,GlOperationError> {
        if let Err(error)=&result {record(self.context,*error);}
        let error=self.error();self.close();
        if error.any() {Err(error)} else {result}
    }
    fn close(&mut self) {
        if !self.live {return;}
        ERRORS.with(|cell| {
            let mut s=cell.get(); s.depth -= 1;
            if s.depth == 0 {s=Ledger::EMPTY;}
            cell.set(s);
        });
        self.live=false;
    }
}
impl Drop for ScopeGuard {fn drop(&mut self) {self.close();}}

/// Bound to this thread and this exact GL context. Nested operations on the
/// same context share evidence; a second context fails before executing work.
pub struct GlErrorScope<'a> {gl: &'a glow::Context, guard: ScopeGuard}
impl<'a> GlErrorScope<'a> {
    pub fn begin(gl: &'a glow::Context) -> Result<Self, GlOperationError> {
        let scope=Self {gl, guard: ScopeGuard::enter(key(gl))?};
        drain_errors(gl, |_| {});
        let error=scope.guard.error();
        if error.any() {return Err(error);}
        Ok(scope)
    }
    pub fn finish<T>(mut self, result: Result<T, GlOperationError>) -> Result<T, GlOperationError> {
        drain_errors(self.gl, |_| {});
        self.guard.complete(result)
    }
}
#[allow(unsafe_code)] // Same narrowly scoped GL read as the existing diagnostic helper.
pub(crate) fn drain_errors(gl: &glow::Context, report: impl FnMut(u32)) {
    drain_with(key(gl), ||unsafe {gl.get_error()}, report);
}
fn drain_with(context: usize, mut read: impl FnMut()->u32, mut report: impl FnMut(u32)) {
    let mut seen=GlOperationError::default();
    loop {
        let code=read();
        if code == glow::NO_ERROR {return;}
        let error=GlOperationError::from_gl_code(code);
        record(context,error); report(code);
        if seen.0 & error.0 != 0 {
            record(context,GlOperationError(GlOperationError::REPEATED)); return;
        }
        seen.0 |= error.0;
        if error.0 & GlOperationError::LOST != 0 {return;}
    }
}

#[cfg(test)]
#[path = "error_scope_tests.rs"]
mod tests;
