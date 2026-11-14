# Recursive calls yield only on direct recursion outside warp mode

TurboWarp's compiler yields before a procedure calls *itself* when the call is not in warp mode (`yieldForRecursion` in jsgen.js), before the arguments are evaluated. Indirect recursion (A calls B calls A) does not yield, unlike scratch-vm's `Thread.isRecursiveCall`, which looks six stack levels down. scrust emits `yield_now()` before such a call.
