# `switch backdrop to () and wait` aborts the tick (bug compatibility)

`_setBackdrop` calls `runtime.startHats` directly instead of `util.startHats`. Starting a hat thread re-targets the shared `BlockUtility`, so the following `util.stackFrame.startedThreads.length` reads the new thread's frame and throws a `TypeError`. The exception unwinds `Runtime._step`; the browser's timer keeps calling `_step`, and on the next tick the block finds its saved thread list and waits normally.

Observable effect: when the block starts at least one hat, the rest of that tick (remaining threads of the pass, further passes) is lost. scrust reproduces it with `World::abort_tick`. Covered by the `backdrops` cases; the reference tracer swallows the exception like a browser does.
