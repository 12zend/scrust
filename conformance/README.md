# Conformance and benchmarks

Everything here compares scrust with the pinned reference implementation (`REFERENCE.lock`).

## Setup

```bash
git clone --depth 1 -b develop https://github.com/scratchfoundation/scratch-vm.git conformance/vendor/scratch-vm
```

```bash
(cd conformance/vendor/scratch-vm && npm install --omit=dev --ignore-scripts)
```

(`conformance/vendor` is git-ignored. TurboWarp is only needed for the benchmark: clone `TurboWarp/scratch-vm` to `conformance/vendor/tw-scratch-vm` and install it the same way.)

## Differential traces

```bash
node conformance/harness/run.js
```

Builds every case in `tests/*.js` with the sb3 DSL (`harness/sb3.js`), traces it in scratch-vm (`harness/trace.js`) and in scrust (`scrust trace`), and compares the JSON-lines output byte for byte. Each case runs twice: `--no-unbox` (exact, type tags included) and unboxed (canonical numeric strings equal their numbers). Options: `--only <substr>`, `--mode nounbox|unbox|both|interp|all` (`interp`: the IR interpreter, `scrust trace --interp`), `--ref scratch-vm|compiled`, `--jobs N`, `--release`.

`--ref compiled` takes the compiled `--no-unbox` program as the reference instead of scratch-vm, so it needs no scratch-vm checkout; it checks the interpreter against the compiler:

```bash
node conformance/harness/run.js --ref compiled --mode interp --release
```

The fuzzer scales with environment variables:

```bash
FUZZ_PROJECTS=200 FUZZ_SEED=1 node conformance/harness/run.js --only 03_
```

`node conformance/harness/bisect.js <seed>` finds the first statement of a failing fuzz project.

## Number / string conversions against Node

```bash
cargo build --release -p scrust-value --example crosscheck
```

```bash
node conformance/value/gen.js 1000000 1 | ./target/release/examples/crosscheck
```

## Renderer-dependent behaviour

```bash
node conformance/render/compare.js
```

Compares fencing, size clamping, `touching` and bounce results with `render/expected.json`. The golden values come from the real scratch-vm + scratch-render (WebGL): `node conformance/render/make.js` writes `out/rendertest/index.html` and `project.sb3`; serve that directory over HTTP, open the page and copy the JSON it prints (also `window.__result`) into `expected.json`.

## Benchmarks

```bash
node conformance/bench/run.js
```

Real projects against TurboWarp's compiler (headless, deterministic, scripts only; native and wasm builds; min of `--repeat` runs), and a trace check of optimised builds against another checkout's builds:

```bash
node conformance/bench/real.js /path/to/sb3/dir --frames 150 --repeat 2
```

```bash
node conformance/bench/verify.js /path/to/sb3/dir --ref /path/to/other/checkout --frames 60
```

## Demo

`node conformance/demo/make.js` writes `conformance/out/demo.sb3` (SVG costumes, pen, clones, effects, sound).

## TurboWarp reference (real projects, with a renderer)

`conformance/twref` runs a TurboWarp scratch-vm checkout with TurboWarp's scratch-render in a browser, deterministically (same clock, seed and one pass per frame as `harness/trace.js`), and saves trace lines and screenshots. Bundle the page (uses the vendored scratch-gui's packages; `SCRUST_REF_VM` defaults to `/Users/kobajin/repo/scratch-vm-develop`):

```bash
(cd conformance/twref && ../../gui/vendor/scratch-gui/node_modules/.bin/webpack --config webpack.config.js)
```

Serve it (`.claude/launch.json` has it as `scrust-twref`) and open `http://localhost:8768/?list=*&frames=300&traceEvery=30&every=30` in a browser; results go to `conformance/out/twref/results`:

```bash
node conformance/twref/serve.js 8768 /path/to/sb3/dir
```

Then build and run the same projects with scrust and compare (traces line by line, screenshots by mean pixel difference):

```bash
node conformance/twref/run.js scrust /path/to/sb3/dir --frames 300 --trace-every 30 --every 30
```

```bash
node conformance/twref/run.js compare /path/to/sb3/dir --pixels
```

A built program accepts `--trace-every N` and `--screenshot-every N --screenshot out-%d.png` for this.
