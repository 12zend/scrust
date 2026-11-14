# Inputs of waiting blocks are evaluated once

scratch-vm's interpreter waits by executing the same block again on every step and re-evaluates the block's inputs each time, which shows up as extra random numbers. TurboWarp's compiler evaluates them once: `repeat` reads its count once, `wait`, `broadcast and wait` and `procedures_call` evaluate their inputs once, and blocks run through the compatibility layer (`glide`, `glide to`, `switch backdrop and wait`, `say/think for secs`, `ask and wait`, `play sound until done`) receive one inputs object and only re-run the primitive. scrust follows TurboWarp. `glide to (random position)` therefore still draws a new target on every step (the primitive calls `getTargetXY` each time), and `wait until` / `repeat until` re-evaluate their condition as part of what they do.

Inputs are evaluated in the order of the generated JavaScript, i.e. argument order (`letter of` evaluates the string first); see [turbowarp.md](turbowarp.md).
