//! Turns the MFT byte stream, read span by span in MFT order, into record
//! segments: whole records are parsed in parallel, a record split across
//! two extents (clusters smaller than records) is assembled first.
use super::index::{MftIndex, MftIndexBuilder};
use super::layout::NtfsVolumeData;
use super::record::{parse_record, RecordDefect};
use super::UNFLUSHED_NOTE;
use rayon::prelude::*;
use std::io;

pub struct MftRecordReader {
    record_size: usize,
    records: u64,
    next_logical: u64,
    carry: Vec<u8>,
    /// Bytes of a record whose start lay in a sparse hole: dropped.
    discard: usize,
    builder: MftIndexBuilder,
}

impl MftRecordReader {
    pub fn new(data: &NtfsVolumeData) -> io::Result<Self> {
        let record_size = data.bytes_per_record as usize;
        Ok(Self {
            record_size,
            records: data.record_count(),
            next_logical: 0,
            carry: Vec::with_capacity(record_size),
            discard: 0,
            builder: MftIndexBuilder::new(data.record_count())?,
        })
    }

    fn expect(&self, logical: u64) -> io::Result<()> {
        if logical == self.next_logical {
            Ok(())
        } else {
            Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "MFT-Leseblöcke nicht in MFT-Reihenfolge",
            ))
        }
    }

    fn record_number(&self, logical: u64) -> u64 {
        logical / self.record_size as u64
    }

    /// The bytes at MFT offset `logical`, directly after the previous span.
    /// Fixups are applied in place.
    pub fn feed(&mut self, logical: u64, bytes: &mut [u8]) -> io::Result<()> {
        self.expect(logical)?;
        self.next_logical = logical.saturating_add(bytes.len() as u64);
        let mut position = logical;
        let mut rest = bytes;
        if self.discard > 0 {
            let take = self.discard.min(rest.len());
            self.discard -= take;
            position += take as u64;
            rest = std::mem::take(&mut rest).split_at_mut(take).1;
        }
        if !self.carry.is_empty() {
            let take = (self.record_size - self.carry.len()).min(rest.len());
            let (head, tail) = std::mem::take(&mut rest).split_at_mut(take);
            self.carry.extend_from_slice(head);
            position += take as u64;
            rest = tail;
            if self.carry.len() == self.record_size {
                let number = self.record_number(position - self.record_size as u64);
                let mut record = std::mem::take(&mut self.carry);
                let parsed = parse_record(&mut record, number);
                self.add(number, parsed);
                record.clear();
                self.carry = record;
            }
        }
        let whole = rest.len() / self.record_size * self.record_size;
        let (records, remainder) = rest.split_at_mut(whole);
        let first = self.record_number(position);
        let parsed: Vec<_> = records
            .par_chunks_mut(self.record_size)
            .enumerate()
            .map(|(index, record)| {
                let number = first + index as u64;
                (number, parse_record(record, number))
            })
            .collect();
        for (number, result) in parsed {
            self.add(number, result);
        }
        self.carry.extend_from_slice(remainder);
        Ok(())
    }

    /// A sparse hole of `len` bytes at `logical`: its records hold nothing.
    /// A record cut by the hole is a defect.
    pub fn skip(&mut self, logical: u64, len: u64) -> io::Result<()> {
        self.expect(logical)?;
        self.next_logical = logical.saturating_add(len);
        let size = self.record_size as u64;
        if !self.carry.is_empty() {
            let number = self.record_number(logical - self.carry.len() as u64);
            self.carry.clear();
            self.add(number, Err(RecordDefect::Header));
        }
        let tail = self.next_logical % size;
        if tail != 0 {
            self.discard = (size - tail) as usize;
            let number = self.record_number(self.next_logical);
            self.add(number, Err(RecordDefect::Header));
        }
        Ok(())
    }

    /// The volume cache could not be flushed before reading: records of
    /// files changed moments ago may still be old on disk. Not partial:
    /// every record read is complete, only its age is uncertain.
    pub fn note_unflushed(&mut self, error: &io::Error) {
        self.builder
            .note(format!("{UNFLUSHED_NOTE} (Cache leeren: {error})"));
    }

    fn add(&mut self, number: u64, parsed: Result<super::RecordState, RecordDefect>) {
        if number < self.records {
            self.builder.add(number, parsed);
        }
    }

    /// The index; a record left incomplete at the end counts as a defect.
    pub fn finish(mut self) -> MftIndex {
        if !self.carry.is_empty() {
            let number = self.record_number(self.next_logical - self.carry.len() as u64);
            self.add(number, Err(RecordDefect::Header));
        }
        self.builder.finish()
    }
}
