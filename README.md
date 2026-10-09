# scrust

Scratch 3 → Rust ahead-of-time compiler. `scrust` reads a `.sb3`, generates a Cargo crate and builds it into a native executable or a WebAssembly page. The specification is [PRODUCT.md](PRODUCT.md); the reference for behaviour is vanilla scratch-vm at the commits in [REFERENCE.lock](REFERENCE.lock).

```
project.sb3 ─parse─▶ AST ─lower─▶ IR ─analysis─▶ typed IR ─codegen─▶ Rust crate ─cargo─▶ exe / wasm
```

## Usage

```bash
cargo build --release -p scrust-cli
```

```bash
./target/release/scrust run project.sb3
```

| Command | What it does |
|---|---|
| `scrust build <in.sb3> [-o out/] [--target native\|wasm] [--single-html]` | build an executable, or `index.html` + `.wasm` |
| `scrust run <in.sb3> [-- program args]` | build and run in a window (`Esc` = stop, `F5` = green flag) |
| `scrust emit <in.sb3> -o crate/` | write the generated crate only |
| `scrust check <in.sb3> [--types]` | unsupported blocks; storage types and procedure variants |
| `scrust trace <in.sb3> --frames N` | deterministic headless run, JSON-lines trace |

Options: `--fps N` (default 30), `--turbo`, `--warp-timer` / `--compat=strict`, `--deterministic --seed N`, `--no-unbox`, `--allow-unsupported`, `--headless`, `--max-passes N`, `--answers 'a|b'`. A built program accepts `--scale S`, `--frames N`, `--screenshot out.png`, `--trace N`, `--print-say`, `--bench`. On macOS it draws with Metal; `SCRUST_RENDERER=cpu` forces the CPU compositor (deterministic runs use it unless `SCRUST_RENDERER=metal`).

## GUI

[gui/](gui/README.md) is TurboWarp's scratch-gui in Electron with scratch-vm's execution and scratch-render replaced by scrust: the green flag compiles the project to a native program and draws its frames on the stage. See [docs/decisions/gui.md](docs/decisions/gui.md).

## Crates

| Crate | Role |
|---|---|
| `scrust-value` | `Value`, `JsStr` (UTF-16, Latin-1/UTF-16 hybrid, rope), ECMAScript number⇄string, casts, compare, math |
| `scrust-sb3` | `.sb3` / `project.json` loader (JS key order, typed literals) |
| `scrust-ir` | lowering to a name-resolved structured IR |
| `scrust-analysis` | type lattice, storage types, kind-sensitivity, representations, yield analysis |
| `scrust-codegen` | IR → Rust (typed expressions, async/sync procedure variants) |
| `scrust-runtime` | scheduler, world, block primitives, trace |
| `scrust-render` | Metal (macOS) and CPU compositors, collision, pen, bubbles, monitors |
| `scrust-player` | native window + audio, web host, embedded host for the GUI (`--embed`) |
| `scrust-node` | Node-API addon over `scrust-render`'s scene, the GUI's stage renderer |
| `scrust-cli` | the `scrust` command |

## Status

| Phase (PRODUCT.md §16) | State |
|---|---|
| 0 values | done: spec table, 10 M random `Number()`/`String()` cases identical to Node |
| 1 front end, scheduler, headless | done: traces identical to scratch-vm on the conformance suite (control, operators, data, events, procedures, clones, motion/looks state) |
| 2 type inference, unboxing | done: storage + flow types, kind-sensitivity, sync/async variants; `--no-unbox` and unboxed runs agree; 1.1×–6.4× faster than TurboWarp on the bundled benchmarks |
| 3 rendering, input | working; fencing, size clamping and the `touching` blocks match scratch-render on the golden test (`conformance/render`, 78/78); drawing on the GPU with Metal on macOS (sprites, effects, pen), CPU compositor elsewhere and for collisions; no pixel-level comparison with scratch-render |
| 4 pen, sound | working (pen, WAV/MP3 playback, pitch/pan/volume); no pixel or timing comparison with the reference |
| 5 wasm | working: `--target wasm`, `--single-html` |
| 6 optional | not started (music, cloud variables, interpolation, sb2) |

Known gaps are listed in [docs/decisions](docs/decisions) (one file per decision); the largest are in `rendering.md`.

## Tests

```bash
cargo test --workspace
```

Differential tests, the fuzzer and the benchmarks need Node and a scratch-vm checkout: see [conformance/README.md](conformance/README.md).
