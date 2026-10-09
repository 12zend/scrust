# Deterministic mode and the reference tracer

`--deterministic` uses a virtual clock (`2020-01-01T00:00:00Z` advancing `1000/fps` ms per tick, frozen inside a tick), a seeded mulberry32 generator returning `u32 / 2^32`, and a bound on scheduler passes per tick (`--max-passes`, default 1) instead of the 75 % work-time budget.

`conformance/harness/trace.js` puts scratch-vm in the same regime: `Date.now`, `Math.random` and `setTimeout` are replaced, the sequencer's work timer counts passes, and passes are counted through the profiler hooks. scratch-vm's `util/uid` draws ids for targets, clones and say bubbles from `Math.random`; the tracer replaces it with a counter so that the seeded stream only contains draws a project can observe. Both sides report the number of RNG draws per frame, so hidden desynchronisation is caught immediately.

Trace lines carry type tags and distinguish `-0`, `NaN` and infinities.
