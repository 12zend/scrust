# Re-executed blocks re-evaluate their inputs

scratch-vm implements waiting by executing the same block again on every step, and `execute()` evaluates the block's inputs each time. Reporters have no side effects except drawing random numbers, so this is only visible in the random sequence. For bit-exact deterministic traces scrust re-evaluates RNG-consuming inputs of `repeat` (every iteration), `wait`, `wait until`, `repeat until`, `glide`, `broadcast and wait` and `switch backdrop and wait`. `glide to (random position)` re-draws its target on every step for the same reason.

Inputs are evaluated in the order of the block's `inputs` object (JS key order); block-internal RNG use (list index `random`/`any`) happens after all inputs.
