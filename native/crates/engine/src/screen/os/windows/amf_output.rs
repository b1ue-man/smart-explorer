//! The output of one AMF picture (split from `amf_encode.rs`, review
//! 2026-11-19 round 10): its access unit, whether it is a key frame, and
//! what it did with the long-term references. The AMF SDK reports both on
//! the output (`VideoEncoderVCE.h`, `VideoEncoderHEVC.h`,
//! `VideoEncoderAV1.h`: `MarkedLTRIndex`, -1 for none, and
//! `ReferencedLTRIndexBitfield`, with the codec prefixes): a planned mark
//! the encoder did not take leaves its slot unknown, a reported one is
//! recorded with this picture's id, and a picture forced to a confirmed
//! reference that used none and is no key frame is not sent: the next
//! picture is an IDR of this encoder (`key_next`). A runtime that reports
//! neither keeps the planned slots (as before).
use super::*;

impl AmfSession {
    /// The access unit of `data` to `deliver`; whether it is a key frame.
    pub(super) fn take_output(
        &mut self,
        data: This,
        frame_id: u64,
        deliver: &mut dyn FnMut(&[u8]) -> io::Result<()>,
    ) -> io::Result<bool> {
        let mut raw = std::ptr::null_mut();
        // SAFETY: a live data object; one new buffer reference.
        let result = unsafe {
            (vtable::<InterfaceVtbl>(data).query_interface)(data, &IID_AMF_BUFFER, &mut raw)
        };
        let buffer = Owned(created(result, raw, "QueryInterface(AMFBuffer)")?);
        // SAFETY: a live buffer.
        let calls = unsafe { vtable::<BufferVtbl>(buffer.0) };
        // SAFETY: plain queries of the live buffer.
        let (native, length) =
            unsafe { ((calls.get_native)(buffer.0), (calls.get_size)(buffer.0)) };
        if native.is_null() || length == 0 {
            return Err(io::Error::other("AMF gave an empty access unit"));
        }
        // SAFETY: the buffer holds `length` bytes while it is referenced.
        deliver(unsafe { std::slice::from_raw_parts(native.cast::<u8>(), length) })?;
        let int = |name: &str| {
            let mut value = AmfVariant::empty();
            let name = wide(name);
            // SAFETY: a live buffer; an integer property needs no clearing.
            let result =
                unsafe { (calls.data.storage.get_property)(buffer.0, name.as_ptr(), &mut value) };
            (result == AMF_OK).then(|| value.as_int64()).flatten()
        };
        let key = int(self.names.output_type) == Some(self.names.output_key);
        let (_, marking, forced) = self.asked;
        let (marked, used) = (int(self.names.marked_ltr), int(self.names.referenced_ltr));
        let Some(ltr) = self.ltr.as_mut() else {
            return Ok(key);
        };
        if let Some(asked) = marking.filter(|m| marked.is_some_and(|i| i != i64::from(*m))) {
            ltr.unmark(asked as usize);
        }
        if let Some(index) = marked.filter(|index| *index >= 0) {
            if !usize::try_from(index).is_ok_and(|index| ltr.marked(index, frame_id)) {
                self.ltr = None;
                return Ok(key);
            }
        }
        if let (Some(bits), Some(used)) = (forced, used) {
            if used & i64::from(bits) == 0 && !key {
                self.ltr = None;
                self.key_next = true;
                return Err(io::Error::new(
                    io::ErrorKind::WouldBlock,
                    "AMF did not reference the confirmed long-term picture",
                ));
            }
        }
        Ok(key)
    }
}
