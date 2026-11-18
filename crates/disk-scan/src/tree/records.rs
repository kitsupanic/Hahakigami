//! Bounded record allocations: growing a snapshot never copies its existing full chunks.
use super::Record;
use std::ops::{Index, IndexMut};

pub(super) const CHUNK_SIZE: usize = 4096;

#[derive(Debug, Default)]
pub(crate) struct Records {
    chunks: Vec<Vec<Record>>,
    length: usize,
    capacity: usize,
}
impl Clone for Records {
    fn clone(&self) -> Self {
        let chunks: Vec<Vec<Record>> = self.chunks.clone();
        let capacity = chunks.iter().map(Vec::capacity).sum::<usize>();
        Self {
            chunks,
            length: self.length,
            capacity,
        }
    }
}
impl Records {
    pub fn len(&self) -> usize {
        self.length
    }
    pub fn capacity(&self) -> usize {
        self.capacity
    }
    pub fn header_storage(&self) -> (usize, usize) {
        let size = std::mem::size_of::<Vec<Record>>();
        (self.chunks.len() * size, self.chunks.capacity() * size)
    }
    pub fn get(&self, index: usize) -> Option<&Record> {
        self.chunks.get(index / CHUNK_SIZE)?.get(index % CHUNK_SIZE)
    }
    pub fn get_mut(&mut self, index: usize) -> Option<&mut Record> {
        self.chunks
            .get_mut(index / CHUNK_SIZE)?
            .get_mut(index % CHUNK_SIZE)
    }
    pub fn as_ptr(&self) -> *const Record {
        self.chunks.first().map_or(std::ptr::null(), Vec::as_ptr)
    }
    pub fn push(&mut self, record: Record) {
        if self.length.is_multiple_of(CHUNK_SIZE) {
            let chunk = Vec::with_capacity(if self.length == 0 { 4 } else { CHUNK_SIZE });
            self.capacity += chunk.capacity();
            self.chunks.push(chunk);
        }
        let chunk = self.chunks.last_mut().expect("chunk created before append");
        if chunk.len() == chunk.capacity() {
            let old = chunk.capacity();
            let target = (old.max(4) * 2).min(CHUNK_SIZE);
            chunk.reserve_exact(target - chunk.len());
            self.capacity += chunk.capacity() - old;
        }
        chunk.push(record);
        self.length += 1;
    }
}
impl Index<usize> for Records {
    type Output = Record;
    fn index(&self, index: usize) -> &Record {
        self.get(index).expect("record index out of bounds")
    }
}
impl IndexMut<usize> for Records {
    fn index_mut(&mut self, index: usize) -> &mut Record {
        self.get_mut(index).expect("record index out of bounds")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn record(inode: u64) -> Record {
        Record {
            device: 1,
            inode,
            allocated_size: 3,
            length: 2,
            own_allocated_size: 3,
            name_offset: 0,
            children_offset: 0,
            parent: 0,
            name_len: 0,
            children_len: 0,
            flags: 0,
        }
    }
    #[test]
    fn bounded_growth_keeps_full_chunk_payloads_and_checks_bounds() {
        let mut records = Records::default();
        assert!(records.get(0).is_none());
        for id in 0..CHUNK_SIZE {
            records.push(record(id as u64));
        }
        let first = records.as_ptr();
        for id in CHUNK_SIZE..(CHUNK_SIZE * 257 + 17) {
            records.push(record(id as u64));
        }
        assert_eq!(records.as_ptr(), first);
        assert_eq!(records.len(), CHUNK_SIZE * 257 + 17);
        assert!(records.capacity() - records.len() < CHUNK_SIZE);
        assert_eq!(
            records.capacity(),
            records.chunks.iter().map(Vec::capacity).sum::<usize>()
        );
        assert!(records.chunks.iter().all(|c| c.capacity() <= CHUNK_SIZE));
        assert!(records.get(records.len()).is_none());
        for id in [
            CHUNK_SIZE - 1,
            CHUNK_SIZE,
            CHUNK_SIZE + 1,
            records.len() - 1,
        ] {
            assert_eq!(records[id].inode, id as u64);
            records[id].allocated_size = 99;
            assert_eq!(records[id].allocated_size, 99);
        }
    }
    #[test]
    fn partial_clone_is_independent_and_append_remains_bounded() {
        let mut original = Records::default();
        for id in 0..CHUNK_SIZE + 3000 {
            original.push(record(id as u64));
        }
        let mut copy = original.clone();
        assert_ne!(copy.as_ptr(), original.as_ptr());
        copy[CHUNK_SIZE].inode = 7;
        assert_eq!(original[CHUNK_SIZE].inode, CHUNK_SIZE as u64);
        for id in copy.len()..CHUNK_SIZE * 3 + 1 {
            copy.push(record(id as u64));
        }
        assert_eq!(
            copy.capacity(),
            copy.chunks.iter().map(Vec::capacity).sum::<usize>()
        );
        assert!(copy.chunks.iter().all(|c| c.capacity() <= CHUNK_SIZE));
        assert!(copy.capacity() - copy.len() < CHUNK_SIZE);
        assert_eq!(copy[CHUNK_SIZE * 2].inode, (CHUNK_SIZE * 2) as u64);
        assert_eq!(original.len(), CHUNK_SIZE + 3000);
    }
    #[test]
    #[should_panic(expected = "record index out of bounds")]
    fn mutation_cannot_cross_logical_length() {
        let mut records = Records::default();
        records.push(record(0));
        records[1].inode = 2;
    }
}
