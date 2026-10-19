use super::model::Row;
use anyhow::{Result, ensure};
use std::{
    fs::File,
    io::{Read, Seek, SeekFrom, Write},
    ops::Range,
};
pub const PAGE_ROWS: usize = 256;
pub const PREFETCH: usize = 128;
pub const MAX_CACHE: usize = 256;

pub fn visible_range(offset: f64, height: f64, row_height: f64, total: usize) -> Range<usize> {
    let first = (offset.max(0.0) / row_height) as usize;
    first.saturating_sub(24)..((first + (height / row_height).ceil() as usize + 25).min(total))
}
/// Anonymous temporary storage disappears on exit, including after a crash.
/// Only compact offsets stay in the worker; UI row metadata stays viewport-sized.
pub struct Store {
    file: File,
    offsets: Vec<u64>,
}
impl Store {
    pub fn new() -> Result<Self> {
        Ok(Self { file: tempfile::tempfile()?, offsets: Vec::new() })
    }
    pub fn len(&self) -> usize {
        self.offsets.len()
    }
    pub fn is_empty(&self) -> bool {
        self.offsets.is_empty()
    }
    pub fn push(&mut self, row: &Row) -> Result<()> {
        let bytes = serde_json::to_vec(row)?;
        let offset = self.file.seek(SeekFrom::End(0))?;
        self.file.write_all(&(bytes.len() as u64).to_le_bytes())?;
        self.file.write_all(&bytes)?;
        self.offsets.push(offset);
        Ok(())
    }
    pub fn read(&mut self, index: usize) -> Result<Row> {
        let offset = *self.offsets.get(index).ok_or_else(|| anyhow::anyhow!("History row is not loaded"))?;
        self.file.seek(SeekFrom::Start(offset))?;
        let mut len = [0; 8];
        self.file.read_exact(&mut len)?;
        let len = u64::from_le_bytes(len) as usize;
        ensure!(len <= 16 * 1024 * 1024, "History cache record is too large");
        let mut bytes = vec![0; len];
        self.file.read_exact(&mut bytes)?;
        Ok(serde_json::from_slice(&bytes)?)
    }
    pub fn page(&mut self, start: usize, count: usize) -> Result<Vec<Row>> {
        (start..start.saturating_add(count.min(MAX_CACHE)).min(self.len())).map(|i| self.read(i)).collect()
    }
}
