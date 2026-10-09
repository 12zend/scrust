# Promise blocks skip the loop-end yield

When a promise-returning block (`say/think for secs`, `ask and wait`, `play sound until done`) resolves, scratch-vm pops the stack until it finds a next block or a loop frame and pushes a null block. If the promise block was the last statement of a loop body (also through enclosing `if`s and procedure returns), the next step re-executes the loop block immediately: that iteration's end-of-loop yield never happens.

Generated code sets a per-thread flag after such a block in tail position; the loop-end yield consumes it. Calls to procedures that may end on a promise clear the flag when the call is not itself in tail position.
