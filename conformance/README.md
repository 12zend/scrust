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

Builds every case in `tests/*.js` with the sb3 DSL (`harness/sb3.js`), traces it in scratch-vm (`harness/trace.js`) and in scrust (`scrust trace`), and compares the JSON-lines output byte for byte. Each case runs twice: `--no-unbox` (exact, type tags included) and unboxed (canonical numeric strings equal their numbers). Options: `--only <substr>`, `--mode nounbox|unbox|both`, `--jobs N`, `--release`.

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

## Demo

`node conformance/demo/make.js` writes `conformance/out/demo.sb3` (SVG costumes, pen, clones, effects, sound).
