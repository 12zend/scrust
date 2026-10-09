# Recursive calls yield outside warp mode

`Sequencer.stepToProcedure` yields when `Thread.isRecursiveCall` is true and neither the caller nor the callee is in warp mode. `isRecursiveCall` only inspects the six stack levels below the call block, and every enclosing control block is one level. A recursive call nested inside six or more `if`/loop blocks is therefore *not* detected and does not yield.

scrust tracks, per thread, the chain of non-warp calls into procedures that lie on a call cycle as `(procedure, nesting depth of the call site)` and reproduces the six-level window (`World::enter_call`). Covered by the `procs` cases (`rec`, `ping/pong`, `deep`).
