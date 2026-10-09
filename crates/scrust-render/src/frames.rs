//! Frames shared between a program running with `--embed --frames-shm PATH`
//! (the writer) and the GUI's renderer addon (the reader, `crates/scrust-node`)
//! through a memory-mapped file, instead of being piped through the Electron
//! main process.
//!
//! Layout (native endianness, both sides run on the same machine):
//!
//! | offset | field |
//! |---|---|
//! | 0 | magic `u32` (`MAGIC`), version `u32` |
//! | 8 | slot capacity in bytes, `u64` |
//! | 16 | frames published so far, `u64` (0: none yet) |
//! | 24 | slot holding the latest frame, `u64` |
//! | 32, 40 | sequence number of slot 0 and 1, `u64` (odd while being written) |
//! | 48, 56 | width and height `u32` of slot 0, then of slot 1 |
//! | 64 | slot 0 pixels, then slot 1 pixels (RGBA, `capacity` bytes each) |
//!
//! The writer fills the slot that does not hold the latest frame, so a reader
//! copying the latest frame is only disturbed when two newer frames arrive
//! during its copy; each slot is a seqlock and the reader retries then.

use std::fs::{File, OpenOptions};
use std::path::Path;
use std::ptr::NonNull;
use std::sync::atomic::{fence, AtomicU32, AtomicU64, Ordering};

const MAGIC: u32 = 0x4652_4353; // "SCRF"
const VERSION: u32 = 1;
const HEADER: usize = 64;

struct Map {
    ptr: NonNull<u8>,
    len: usize,
}

impl Map {
    fn new(file: &File, len: usize, write: bool) -> Option<Map> {
        use std::os::fd::AsRawFd;
        let prot = if write { libc::PROT_READ | libc::PROT_WRITE } else { libc::PROT_READ };
        // SAFETY: a fresh shared mapping of `len` bytes of an open file; it is
        // only unmapped in `drop`.
        let p = unsafe { libc::mmap(std::ptr::null_mut(), len, prot, libc::MAP_SHARED, file.as_raw_fd(), 0) };
        if p == libc::MAP_FAILED {
            return None;
        }
        Some(Map { ptr: NonNull::new(p as *mut u8)?, len })
    }

    fn u32(&self, offset: usize) -> &AtomicU32 {
        debug_assert!(offset + 4 <= HEADER && offset % 4 == 0);
        // SAFETY: inside the header, aligned (mappings are page aligned), and
        // only ever accessed atomically.
        unsafe { &*(self.ptr.as_ptr().add(offset) as *const AtomicU32) }
    }

    fn u64(&self, offset: usize) -> &AtomicU64 {
        debug_assert!(offset + 8 <= HEADER && offset % 8 == 0);
        // SAFETY: as in `u32`.
        unsafe { &*(self.ptr.as_ptr().add(offset) as *const AtomicU64) }
    }

    fn slot(&self, i: usize, capacity: usize) -> *mut u8 {
        // SAFETY: the mapping is HEADER + 2 * capacity bytes long.
        unsafe { self.ptr.as_ptr().add(HEADER + i * capacity) }
    }
}

impl Drop for Map {
    fn drop(&mut self) {
        // SAFETY: `ptr` / `len` describe the mapping made in `new`.
        unsafe { libc::munmap(self.ptr.as_ptr() as *mut libc::c_void, self.len) };
    }
}

fn seq_offset(slot: usize) -> usize {
    32 + slot * 8
}

fn size_offset(slot: usize) -> usize {
    48 + slot * 8
}

/// The program's end: publishes frames of at most `capacity` bytes.
pub struct FrameWriter {
    map: Map,
    capacity: usize,
}

// SAFETY: the mapping is owned by the writer and shared memory is reached
// through atomics and raw copies only.
unsafe impl Send for FrameWriter {}

impl FrameWriter {
    pub fn create(path: &Path, capacity: usize) -> std::io::Result<FrameWriter> {
        let file = OpenOptions::new().read(true).write(true).create(true).truncate(true).open(path)?;
        file.set_len((HEADER + 2 * capacity) as u64)?;
        let map = Map::new(&file, HEADER + 2 * capacity, true).ok_or_else(std::io::Error::last_os_error)?;
        map.u64(8).store(capacity as u64, Ordering::Relaxed);
        map.u32(4).store(VERSION, Ordering::Relaxed);
        // The magic marks the header as complete.
        map.u32(0).store(MAGIC, Ordering::Release);
        Ok(FrameWriter { map, capacity })
    }

    /// Is `rgba` (`width` x `height`) the frame published last?
    pub fn is_latest(&self, width: u32, height: u32, rgba: &[u8]) -> bool {
        if self.map.u64(16).load(Ordering::Relaxed) == 0 {
            return false;
        }
        let i = self.map.u64(24).load(Ordering::Relaxed) as usize;
        let size = self.map.u64(size_offset(i)).load(Ordering::Relaxed);
        if size != (width as u64) | ((height as u64) << 32) || rgba.len() > self.capacity {
            return false;
        }
        // SAFETY: the slot holds `capacity` bytes and only this writer writes it.
        let slot = unsafe { std::slice::from_raw_parts(self.map.slot(i, self.capacity), rgba.len()) };
        slot == rgba
    }

    /// Publish a frame; frames larger than the capacity are dropped.
    pub fn publish(&mut self, width: u32, height: u32, rgba: &[u8]) {
        if rgba.len() > self.capacity || rgba.len() != width as usize * height as usize * 4 {
            return;
        }
        let published = self.map.u64(16).load(Ordering::Relaxed);
        let latest = self.map.u64(24).load(Ordering::Relaxed) as usize;
        let i = if published == 0 { 0 } else { 1 - latest };
        let seq = self.map.u64(seq_offset(i));
        let s = seq.load(Ordering::Relaxed);
        seq.store(s + 1, Ordering::Relaxed);
        fence(Ordering::Release);
        self.map.u64(size_offset(i)).store((width as u64) | ((height as u64) << 32), Ordering::Relaxed);
        // SAFETY: the slot holds `capacity` >= `rgba.len()` bytes.
        unsafe { std::ptr::copy_nonoverlapping(rgba.as_ptr(), self.map.slot(i, self.capacity), rgba.len()) };
        seq.store(s + 2, Ordering::Release);
        self.map.u64(24).store(i as u64, Ordering::Release);
        self.map.u64(16).store(published + 1, Ordering::Release);
    }
}

/// The GUI's end.
pub struct FrameReader {
    map: Map,
    capacity: usize,
}

impl FrameReader {
    /// None until the writer has created the file and its header.
    pub fn open(path: &Path) -> Option<FrameReader> {
        let file = File::open(path).ok()?;
        let len = file.metadata().ok()?.len() as usize;
        if len < HEADER {
            return None;
        }
        let map = Map::new(&file, len, false)?;
        if map.u32(0).load(Ordering::Acquire) != MAGIC || map.u32(4).load(Ordering::Relaxed) != VERSION {
            return None;
        }
        let capacity = map.u64(8).load(Ordering::Relaxed) as usize;
        if HEADER + 2 * capacity > len {
            return None;
        }
        Some(FrameReader { map, capacity })
    }

    /// Number of frames published so far.
    pub fn published(&self) -> u64 {
        self.map.u64(16).load(Ordering::Acquire)
    }

    /// Copy the latest frame into the buffer `alloc(len)` returns, when one
    /// newer than `seen` exists: `(frame number, width, height)`.
    pub fn read(&self, seen: u64, mut alloc: impl FnMut(usize) -> Option<*mut u8>) -> Option<(u64, u32, u32)> {
        let mut out: Option<(*mut u8, usize)> = None;
        // Each retry means two frames were published during the copy; give up
        // after a few (the next poll gets a newer frame anyway).
        for _ in 0..8 {
            let published = self.published();
            if published == 0 || published == seen {
                return None;
            }
            let i = self.map.u64(24).load(Ordering::Acquire) as usize;
            let seq = self.map.u64(seq_offset(i));
            let s1 = seq.load(Ordering::Acquire);
            if s1 % 2 == 1 {
                std::hint::spin_loop();
                continue;
            }
            let size = self.map.u64(size_offset(i)).load(Ordering::Relaxed);
            let (w, h) = (size as u32, (size >> 32) as u32);
            let len = w as usize * h as usize * 4;
            if len > self.capacity {
                return None;
            }
            let dst = match out {
                Some((p, l)) if l == len => p,
                _ => {
                    let p = alloc(len)?;
                    out = Some((p, len));
                    p
                }
            };
            // SAFETY: the slot holds `capacity` >= `len` bytes and `dst` was
            // allocated for `len` bytes. A concurrent write can tear the copy,
            // which the sequence check below detects.
            unsafe { std::ptr::copy_nonoverlapping(self.map.slot(i, self.capacity), dst, len) };
            fence(Ordering::Acquire);
            if seq.load(Ordering::Relaxed) == s1 {
                return Some((published, w, h));
            }
        }
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn frames_round_trip() {
        let path = std::env::temp_dir().join(format!("scrust-frames-test-{}.bin", std::process::id()));
        let mut w = FrameWriter::create(&path, 2 * 2 * 4).expect("writer");
        let r = FrameReader::open(&path).expect("reader");
        let mut buf = vec![0u8; 16];
        let mut read = |seen| r.read(seen, |len| (len == 16).then_some(buf.as_mut_ptr()));
        assert_eq!(read(0), None);
        let a: Vec<u8> = (0..16).collect();
        w.publish(2, 2, &a);
        assert!(w.is_latest(2, 2, &a));
        assert_eq!(read(0), Some((1, 2, 2)));
        assert_eq!(read(1), None);
        let b: Vec<u8> = (100..116).collect();
        assert!(!w.is_latest(2, 2, &b));
        w.publish(2, 2, &b);
        w.publish(2, 2, &a);
        assert_eq!(read(1), Some((3, 2, 2)));
        drop(read);
        assert_eq!(buf, a);
        let _ = std::fs::remove_file(&path);
    }
}
