# Build time: project data as statics

**Decision.** The generated crate keeps project *data* out of function bodies. A list's initial items are a `static` array (`[f64; N]` for unboxed number lists, otherwise `[Lit; N]`, converted with `Elem::from_value` when the list is created), and the costume and sound tables are `static [CostumeDef; N]` / `[SoundDef; N]` copied into the `SpriteDef` (`--assets external` keeps them inline, since `load_asset` runs at start-up).

**Why.** Before, they were `vec![...]` literals inside `new_lN()` and `program()`. rustc lowers each element to code, and LLVM's optimisation time grows faster than linearly with function size. A project with 8 lists of 9 894 numbers, 20 of 2 047 and 4 096 bitmap costumes (`HOUSE.sb3`, 100 MB) spent 49 s of a 55 s release build of the project crate in LLVM (`-Z time-passes`: `LLVM_thinlto`); as statics the same crate builds in 4.6 s, and `scrust build` (dependencies already built) went from 62 s to 5.6 s.

**Equivalence.** `[f64; N]` holds exactly what `E::as_repr(Repr::F64)` produced. For the other representations `Lit` keeps the literal (number, string, string with lone surrogates as UTF-16, boolean) and `Elem::from_value` performs the same conversion the inline expressions did.
