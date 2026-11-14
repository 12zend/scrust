//! The program allocator under a random mix of sizes, alignments and reallocs.
use scrust_runtime::alloc::FastAlloc;
use std::alloc::{GlobalAlloc, Layout};

#[test]
fn random_alloc_free_realloc() {
    let a = FastAlloc;
    let mut seed: u64 = 12345;
    let mut rnd = move || {
        seed ^= seed << 13;
        seed ^= seed >> 7;
        seed ^= seed << 17;
        seed
    };
    let mut live: Vec<(*mut u8, Layout, u8)> = Vec::new();
    for step in 0..200_000u32 {
        let r = rnd();
        if live.len() < 2000 && (r % 3 != 0 || live.is_empty()) {
            let size = 1 + (rnd() % if r % 17 == 0 { 5000 } else { 600 }) as usize;
            let align = [1, 2, 4, 8, 16, 32][(rnd() % 6) as usize];
            let l = Layout::from_size_align(size, align).unwrap();
            let p = unsafe { a.alloc(l) };
            assert!(!p.is_null() && p as usize % align == 0);
            let tag = step as u8;
            unsafe { std::ptr::write_bytes(p, tag, size) };
            live.push((p, l, tag));
        } else {
            let i = (rnd() as usize) % live.len();
            let (p, l, tag) = live.swap_remove(i);
            for k in 0..l.size() {
                assert_eq!(unsafe { *p.add(k) }, tag);
            }
            if r % 5 == 0 {
                let ns = 1 + (rnd() % 700) as usize;
                let q = unsafe { a.realloc(p, l, ns) };
                assert!(!q.is_null() && q as usize % l.align() == 0);
                for k in 0..l.size().min(ns) {
                    assert_eq!(unsafe { *q.add(k) }, tag);
                }
                let nl = Layout::from_size_align(ns, l.align()).unwrap();
                unsafe { std::ptr::write_bytes(q, tag, ns) };
                live.push((q, nl, tag));
            } else {
                unsafe { a.dealloc(p, l) };
            }
        }
    }
    for (p, l, _) in live {
        unsafe { a.dealloc(p, l) };
    }
}
