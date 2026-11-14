//! Global allocator for generated programs.
//!
//! Scratch programs allocate and free huge numbers of small objects (strings,
//! rope nodes, boxed futures of procedure calls). The system allocators are
//! slow at that: macOS's zone allocator reads the clock in `free`, and the
//! wasm32 default (dlmalloc) searches bins. This allocator keeps per-thread
//! free lists of small blocks (size classes of 16 bytes up to `MAX_SMALL`),
//! carved from 256 KiB chunks; larger requests go to the system allocator.
//!
//! A small block freed on another thread joins that thread's lists. Chunks
//! are never returned to the system: the lists are reused by the program.

use std::alloc::{GlobalAlloc, Layout, System};
use std::cell::Cell;
use std::ptr::null_mut;

const CLASS: usize = 16;
const MAX_SMALL: usize = 512;
const CLASSES: usize = MAX_SMALL / CLASS;
const CHUNK: usize = 256 * 1024;

struct Local {
    free: [Cell<*mut u8>; CLASSES],
    /// Bump region of the current chunk.
    next: Cell<*mut u8>,
    end: Cell<*mut u8>,
}

thread_local! {
    static LOCAL: Local = const {
        Local { free: [const { Cell::new(null_mut()) }; CLASSES], next: Cell::new(null_mut()), end: Cell::new(null_mut()) }
    };
}

pub struct FastAlloc;

#[inline(always)]
fn class_of(layout: &Layout) -> Option<usize> {
    if layout.size() <= MAX_SMALL && layout.align() <= CLASS && layout.size() > 0 {
        Some((layout.size() - 1) / CLASS)
    } else {
        None
    }
}

impl FastAlloc {
    #[inline(always)]
    unsafe fn alloc_small(c: usize) -> *mut u8 {
        LOCAL.with(|l| {
            let head = l.free[c].get();
            if !head.is_null() {
                // SAFETY: free blocks start with the pointer to the next one.
                l.free[c].set(*(head as *mut *mut u8));
                return head;
            }
            let size = (c + 1) * CLASS;
            let p = l.next.get();
            if !p.is_null() && (l.end.get() as usize) - (p as usize) >= size {
                l.next.set(p.add(size));
                return p;
            }
            Self::refill(l, size)
        })
    }

    #[cold]
    unsafe fn refill(l: &Local, size: usize) -> *mut u8 {
        // The rest of the old chunk goes onto the free lists.
        let (mut p, end) = (l.next.get(), l.end.get());
        if !p.is_null() {
            while (end as usize) - (p as usize) >= CLASS {
                let rest = (end as usize) - (p as usize);
                let c = (rest.min(MAX_SMALL) / CLASS) - 1;
                let s = (c + 1) * CLASS;
                *(p as *mut *mut u8) = l.free[c].get();
                l.free[c].set(p);
                p = p.add(s);
            }
        }
        let chunk = System.alloc(Layout::from_size_align_unchecked(CHUNK, CLASS));
        if chunk.is_null() {
            l.next.set(null_mut());
            l.end.set(null_mut());
            return null_mut();
        }
        l.next.set(chunk.add(size));
        l.end.set(chunk.add(CHUNK));
        chunk
    }

    #[inline(always)]
    unsafe fn free_small(p: *mut u8, c: usize) {
        LOCAL.with(|l| {
            *(p as *mut *mut u8) = l.free[c].get();
            l.free[c].set(p);
        })
    }
}

unsafe impl GlobalAlloc for FastAlloc {
    #[inline]
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        match class_of(&layout) {
            Some(c) => Self::alloc_small(c),
            None => System.alloc(layout),
        }
    }

    #[inline]
    unsafe fn dealloc(&self, p: *mut u8, layout: Layout) {
        match class_of(&layout) {
            Some(c) => Self::free_small(p, c),
            None => System.dealloc(p, layout),
        }
    }

    #[inline]
    unsafe fn alloc_zeroed(&self, layout: Layout) -> *mut u8 {
        match class_of(&layout) {
            Some(c) => {
                let p = Self::alloc_small(c);
                if !p.is_null() {
                    std::ptr::write_bytes(p, 0, layout.size());
                }
                p
            }
            None => System.alloc_zeroed(layout),
        }
    }

    #[inline]
    unsafe fn realloc(&self, p: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
        let new_layout = Layout::from_size_align_unchecked(new_size, layout.align());
        match (class_of(&layout), class_of(&new_layout)) {
            (None, None) => System.realloc(p, layout, new_size),
            (Some(a), Some(b)) if a == b => p,
            _ => {
                let q = self.alloc(new_layout);
                if !q.is_null() {
                    std::ptr::copy_nonoverlapping(p, q, layout.size().min(new_size));
                    self.dealloc(p, layout);
                }
                q
            }
        }
    }
}
