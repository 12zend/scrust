# Scheduler: transcribed quirks of `Sequencer.stepThreads`

`scrust-runtime::sched` re-implements `Runtime._step`/`stepThreads`/`startHats` semantically. Behaviours that are easy to miss and are covered by `conformance/tests/02_sched.js`:

- A pass iterates the thread list by index and re-reads the length, so threads started during a pass run in the same pass.
- Without turbo mode the tick ends after the pass in which a redraw was requested; otherwise passes repeat until 75 % of the frame time is used (deterministic mode: `--max-passes`).
- `YIELD_TICK` threads resume only in the first pass of the next tick.
- Done threads stay in the list until the end of the pass (so `broadcast and wait` still sees them); killed threads are dropped at the start of the next tick.
- `startHats` with `restartExistingThreads` replaces the thread **in place**. If the replaced thread is the one currently running, it keeps running until its next yield and is then discarded.
- `stop all` replaces `runtime.threads` with a new array while the running pass keeps iterating the old one: threads of non-clone targets that come later in the list get one more step in that pass. Clones are disposed first (their threads are killed).
- Edge-activated hats are evaluated every tick before stepping, `when touching` before `when > `, and do not re-evaluate while a thread of that hat is alive.
- Hats with `restartExistingThreads = true`: flag clicked, sprite clicked, stage clicked, backdrop switches, broadcast received. False: key pressed, start as clone, `when >`, `when touching`.
- Execution order is `executableTargets` from front to back with the stage last; scripts within a target in block (JS key) order.
