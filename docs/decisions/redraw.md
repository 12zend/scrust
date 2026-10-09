# Redraw requests

In non-turbo mode a tick stops stepping threads after the pass in which a redraw was requested (§10.1), so *which* blocks request one decides how far scripts advance per frame. scratch-vm only requests redraws from code paths that have a renderer; `control_wait` is the exception.

| Source | Condition |
|---|---|
| `wait () seconds` (first execution) | always |
| `setXY`, `setDirection`, `setSize`, `setCostume`, `setRotationStyle`, graphic effects, `clear graphic effects` | renderer attached **and** the target is visible |
| `show` | renderer attached (the target is visible afterwards) |
| `hide`, say/think bubble updates (including `ask` questions shown in a bubble) | renderer attached (bubble re-render) |
| layer changes, clone creation / deletion of a visible target | renderer attached |
| pen: `pen down` (dot), line drawn by a move with the pen down, `stamp`, `erase all` | renderer attached |
| monitor show/hide | renderer attached |

`set size` has no effect at all without a renderer (scratch-vm clamps the size inside the `if (this.renderer)` branch), which is why headless traces keep the initial size.

Headless traces (`scrust trace`, the conformance suite) therefore only see redraw requests from `wait`; the windowed player sees all of them.
