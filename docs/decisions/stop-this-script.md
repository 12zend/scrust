# `stop this script` inside a custom block

`Thread.stopThisScript` pops the stack down to the nearest `procedures_call` block, i.e. it returns from the procedure (verified by `procs` cases). The call block is then *executed again* (it is skipped by its `executed` flag), which re-evaluates its inputs. That is only observable through the random number generator, so generated procedures return `true` when left via `stop this script` and call sites with RNG-consuming arguments re-evaluate them.
