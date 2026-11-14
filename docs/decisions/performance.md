# Performance against TurboWarp

**Goal.** Compiled programs, native and WebAssembly, run scripts faster than TurboWarp's JavaScript compiler on real projects, with exactly the same behaviour.

**How it is measured.** `conformance/bench/real.js` runs `.sb3` files in three engines, all headless (no renderer, no audio) and deterministic (virtual clock, seeded random numbers, one scheduler pass per frame, so every engine does the same work per frame): TurboWarp's VM with its compiler in Node (`SCRUST_REF_VM`, the time of `Sequencer.stepThreads`), the native build (`--bench-json`) and the wasm build (`scrust build --target wasm --headless`, run in Node's V8, the engine Chrome uses). The table reports the ratio TurboWarp time / scrust time over the frames every engine completed, and the geometric mean over projects. `conformance/bench/verify.js` checks that optimised builds produce the same per-frame traces as the builds of another checkout, native and wasm (canonical numeric strings and their numbers count as equal, as in `twref`). The dev set is 25 CPU-heavy projects from a 124-project collection (3D engines, ray tracers, emulators, simulations, a transformer).

**What was slow, and what changed.** Profiles of the heavy projects were dominated by the runtime, not by generated arithmetic:

| Cost | Change |
|---|---|
| Numeric strings (list items like `"16777215"`, `".999"`) re-parsed on every read | `JsStr` caches `Number(s)` and `isNotActuallyZero` in the string node; strings are immutable, so the cache never goes stale (V8 does the same for array-index strings) |
| `Cast.toListIndex` comparing every string index against `all` / `last` / `random` / `any` | the cached number is checked first; constant indices (`item "3" of`) are resolved by the code generator |
| Two allocations per string; `malloc`/`free` (macOS's zone allocator reads the clock in `free`, wasm's dlmalloc searches bins) | short Latin-1 strings are stored inline in the node; generated programs use `scrust_runtime::alloc::FastAlloc`, per-thread free lists of 16-byte size classes up to 512 bytes |
| Number to string through `format!` and heap strings | integer fast path, stack buffers, and a 1024-entry cache like V8's number-string cache |
| `Cast.compare` lower-casing copies; formatting a number to compare it with a non-numeric string | case-insensitive comparison in place; a number's string starts with a digit or `-`, so most such comparisons are decided by the other string's first character |
| `item # of` / `contains` running the whole `Cast.compare` per item | the searched value is converted once (`CompareKey`), lengths first |
| Comparisons with constants and string variables building `Value`s | `compare_lit`, `compare_val_str`, `compare_str_ci_lit`, and `letter_eq_ci` for `(letter i of s) = "c"` |
| String constants behind a thread-local (a function call per use on macOS) | `jstr!` constants are lazily filled statics: only the script thread evaluates them |
| Canonical strings like `"0.5"` kept variables boxed when shown by `say` or a monitor | the lattice distinguishes fractions of at most two digits (`str:fract2`): `say` rounds to two decimals and monitors to six, so they show exactly like their numbers |
| Variable storage reached through raw pointers that may alias anything, so LLVM reloads every variable after every store and call | synchronous procedures whose bodies cannot reach their storage through the world (no `(var) of (this sprite or the stage)`, no dynamic `(property) of`, no `create clone`, transitively) receive it as `&mut` parameters (`*_na` functions); `SCRUST_ALIASED_STORAGE=1` turns this off |

Build settings were measured too: fat LTO with one codegen unit, or thin LTO, changed nothing beyond noise (±3 %), so the generated crate keeps `opt-level = 3`, 16 codegen units and no LTO.

**Results** (dev set, 150 frames; geometric mean of TurboWarp time / scrust time):

| | native | wasm |
|---|---|---|
| before | 0.76× | 0.58× |
| after | see the commit introducing this file | |

Per project the range is wide: string- and list-heavy projects (texture loaders, tokenizers, emulators) gain the most, numeric loops the least, because V8 compiles those to code close to LLVM's and both spend the same time in correctly rounded `sin`/`cos` (CORE-MATH natively; V8 calls LLVM libc).

**WebAssembly.** The wasm build runs the same Rust and the same optimisations; the remaining gap to native (about 0.75×) is V8's wasm code generation. Things checked that did not matter: Liftoff → TurboFan tier-up (`--no-liftoff` is not faster), LTO. Variable-length copies become `memory.copy`, which V8 executes out of line, so the hot short-string concatenation copies fixed-size blocks. Trigonometry in wasm uses the `libm` crate (see `math-libm.md`), so its last bits can differ from native builds; `verify.js` compares wasm with wasm.

**Profiling.** `SCRUST_PROFILE_NOINLINE=1` keeps every procedure a separate symbol in the generated code.
