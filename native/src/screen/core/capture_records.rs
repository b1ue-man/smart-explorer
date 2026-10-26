//! Encoded records whose byte allocations and Vec headers were admitted first.
use std::{fmt, io, ops::Deref};
use super::capture_admission::{CaptureGrant, CaptureOwner, CaptureSlice};
use super::capture_bytes::AdmittedBytes;
use super::wire::invalid;

pub(crate) struct CapturedRecords {
    records: Vec<Vec<u8>>,
    output: Option<CaptureSlice>,
    metadata: CaptureSlice,
}
impl fmt::Debug for CapturedRecords {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {self.records.fmt(f)}
}
impl PartialEq for CapturedRecords {
    fn eq(&self, other: &Self) -> bool {self.records == other.records}
}
impl Eq for CapturedRecords {}
impl Deref for CapturedRecords {
    type Target = [Vec<u8>];
    fn deref(&self) -> &Self::Target {&self.records}
}
impl<'a> IntoIterator for &'a CapturedRecords {
    type Item=&'a Vec<u8>;
    type IntoIter=std::slice::Iter<'a,Vec<u8>>;
    fn into_iter(self)->Self::IntoIter {self.records.iter()}
}
impl CapturedRecords {
    pub(crate) fn from_allocator(grant: &CaptureGrant, output_capacity: u64, record_capacity: u64,
        allocate: impl FnOnce() -> io::Result<Vec<Vec<u8>>>) -> io::Result<Self> {
        let output = grant.claim(CaptureOwner::CodecOutput, output_capacity)?;
        let headers = record_capacity.checked_mul(std::mem::size_of::<Vec<u8>>() as u64)
            .ok_or_else(|| invalid("capture record metadata overflow"))?;
        let metadata = grant.claim(CaptureOwner::Metadata, headers)?;
        let records = allocate()?;
        let actual = records.iter().try_fold(0u64, |bytes, record| bytes.checked_add(record.capacity() as u64));
        if records.capacity() as u64 > record_capacity || actual.is_none_or(|bytes| bytes > output_capacity) {
            return Err(invalid("record allocator exceeded its prior capacity promise"));
        }
        Ok(Self {records, output:Some(output), metadata})
    }
    pub(crate) fn one(bytes: AdmittedBytes) -> io::Result<Self> {
        let grant = bytes.grant()?;
        let metadata = grant.claim(CaptureOwner::Metadata, std::mem::size_of::<Vec<u8>>() as u64)?;
        let mut records = Vec::new();
        records.try_reserve_exact(1).map_err(|_| io::Error::other("capture record header allocation failed"))?;
        if records.capacity() > 1 {return Err(invalid("record header allocator exceeded prior promise"));}
        let (data, output) = bytes.into_parts();
        records.push(data);
        Ok(Self {records, output:Some(output), metadata})
    }
    pub(crate) fn empty(grant: &CaptureGrant) -> io::Result<Self> {
        Self::from_allocator(grant, 0, 0, || Ok(Vec::new()))
    }
    pub(crate) fn grant(&self) -> io::Result<CaptureGrant> {self.output.as_ref().ok_or_else(||invalid("capture record owner already consumed"))?.grant()}
    pub(crate) fn as_records(&self) -> &[Vec<u8>] {&self.records}
    pub(crate) fn reverse(&mut self) {self.records.reverse();}
    pub(crate) fn pop(&mut self) -> io::Result<Option<AdmittedBytes>> {
        let Some(data) = self.records.pop() else {return Ok(None);};
        let capacity = data.capacity() as u64;
        let output = self.output.take().ok_or_else(||invalid("capture record owner already consumed"))?;
        let (record, remaining) = output.split(capacity)?;
        self.output = Some(remaining);
        Ok(Some(AdmittedBytes::from_admitted_parts(data, record)?))
    }
}
pub(crate) struct CaptureRecordBuilder {
    records: Vec<Vec<u8>>,
    capacity: u64,
    allocated: u64,
    record_capacity: usize,
    output: Option<CaptureSlice>,
    metadata: CaptureSlice,
}
impl CaptureRecordBuilder {
    pub(crate) fn new(grant: &CaptureGrant, capacity: u64, records: u64) -> io::Result<Self> {
        let record_capacity = usize::try_from(records).map_err(|_| invalid("record count exceeds address space"))?;
        let output = grant.claim(CaptureOwner::CodecOutput, 0)?;
        let metadata_bytes = records.checked_mul(std::mem::size_of::<Vec<u8>>() as u64)
            .ok_or_else(|| invalid("record container size overflow"))?;
        let metadata = grant.claim(CaptureOwner::Metadata, metadata_bytes)?;
        let mut records = Vec::new();
        records.try_reserve_exact(record_capacity).map_err(|_| io::Error::other("capture record container allocation failed"))?;
        if records.capacity() > record_capacity {return Err(invalid("record container allocator exceeded prior promise"));}
        Ok(Self {records, capacity, allocated: 0, record_capacity, output:Some(output), metadata})
    }
    pub(crate) fn start(&mut self, wanted: u64, prefix: &[u8]) -> io::Result<()> {
        let capacity = wanted.min(self.capacity - self.allocated);
        if self.records.len() == self.record_capacity || capacity < prefix.len() as u64 {
            return Err(invalid("capture record builder exhausted"));
        }
        let length = usize::try_from(capacity).map_err(|_| invalid("record allocation exceeds address space"))?;
        let grant = self.output.as_ref().ok_or_else(||invalid("capture record owner already consumed"))?.grant()?;
        let promised = grant.claim(CaptureOwner::CodecOutput, capacity)?;
        let old = self.output.take().ok_or_else(||invalid("capture record owner already consumed"))?;
        self.output = Some(old.merge(promised)?);
        let mut data = Vec::new();
        data.try_reserve_exact(length).map_err(|_| io::Error::other("capture record allocation failed"))?;
        if data.capacity() as u64 > capacity {return Err(invalid("record allocator exceeded requested capacity"));}
        data.extend_from_slice(prefix);
        self.allocated += data.capacity() as u64;
        self.records.push(data);
        Ok(())
    }
    pub(crate) fn remaining(&self) -> usize {
        self.records.last().map_or(0, |record| record.capacity() - record.len())
    }
    pub(crate) fn push(&mut self, bytes: AdmittedBytes) -> io::Result<()> {
        let grant = self.output.as_ref().ok_or_else(||invalid("capture record owner already consumed"))?.grant()?;
        if !bytes.belongs_to(&grant) || self.records.len() == self.record_capacity
            || self.allocated.checked_add(bytes.capacity()).is_none_or(|size| size > self.capacity) {
            return Err(invalid("captured record does not fit this admitted container"));
        }
        let (data, slice) = bytes.into_parts();
        if slice.owner() != CaptureOwner::CodecOutput {
            drop(data);
            drop(slice);
            return Err(invalid("captured record has another owner class"));
        }
        let old = self.output.take().ok_or_else(||invalid("capture record owner already consumed"))?;
        self.output = Some(old.merge(slice)?);
        self.allocated += data.capacity() as u64;
        self.records.push(data);
        Ok(())
    }
    pub(crate) fn last_len(&self) -> usize {self.records.last().map_or(0, Vec::len)}
    pub(crate) fn len(&self) -> usize {self.records.len()}
    pub(crate) fn append(&mut self, bytes: &[u8]) -> io::Result<()> {
        let record = self.records.last_mut().ok_or_else(|| invalid("capture record has not started"))?;
        if bytes.len() > record.capacity() - record.len() {return Err(invalid("capture record cannot grow"));}
        record.extend_from_slice(bytes);
        Ok(())
    }
    pub(crate) fn overwrite(&mut self, offset: usize, bytes: &[u8]) -> io::Result<()> {
        let record = self.records.last_mut().ok_or_else(|| invalid("capture record has not started"))?;
        let end = offset.checked_add(bytes.len()).ok_or_else(|| invalid("capture record offset overflow"))?;
        let destination = record.get_mut(offset..end).ok_or_else(|| invalid("capture record overwrite outside initialized bytes"))?;
        destination.copy_from_slice(bytes);
        Ok(())
    }
    pub(crate) fn freeze(self) -> CapturedRecords {
        CapturedRecords {records: self.records, output: self.output, metadata: self.metadata}
    }
}
