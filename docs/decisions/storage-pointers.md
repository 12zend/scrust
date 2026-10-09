# Cached storage pointers in generated code

Generated functions cache a `LocalsPtr` to the stage's and the current target's variable struct instead of resolving `World -> target -> Box<dyn Locals>` on every access (measured 2-3x on variable-heavy loops). This is the one place generated code relies on `unsafe`; the invariants are documented on `LocalsPtr`: the struct is its own heap allocation, never moves, outlives every thread of its target (slots of deleted clones are recycled only when no thread references them), and the runtime does not touch it while generated code holds a reference.

`world()` (the thread-local world pointer used by `async` code) follows PRODUCT.md §10.6: never held across `.await`; synchronous functions receive `&mut World` as a parameter.
