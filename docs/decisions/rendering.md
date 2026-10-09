# Rendering: CPU compositor instead of wgpu

**Decision.** `scrust-render` composites the stage on the CPU (resvg/tiny-skia for SVG and pen strokes, a hand-written sampler for sprites and graphic effects) and the player presents the result with softbuffer (native) or a 2D canvas (web). PRODUCT.md §13 asks for a wgpu renderer; that is not implemented.

**Why.** Collision detection has to be answered on the CPU with the same sampling rules as drawing (§13: "GPU 読み戻しはしない"), so a CPU sampler is needed in any case. Using it for presentation as well gives one code path for native and wasm, identical pixels for `touching color` and the screen, and no GPU or wasm-bindgen dependency. The cost is fill rate: full-screen sprites at high scale are much slower than on a GPU.

**What follows scratch-render.** The model transform (`Drawable._calculateTransform`), AABB (`Rectangle.initFromModelMatrix`), fencing (`getFencedPositionOfDrawable`, 15 px fence), tight bounds from opaque pixel centres, candidate/intersection scan for `touching sprite`, colour matching with the 5/5/4-bit masks and compositing over white for `touching color`, effect formulas (`sprite.frag` / `EffectTransform`), SVG MIP levels at powers of two capped at 2048 px, the pen's half-pixel offset for 1 and 3 px lines.

**Verified against the reference.** `conformance/render` is a project that logs renderer-dependent results (fenced positions for upright and rotated sprites, `set size` clamping, `touching sprite` at 0-2 px distances, rotated and scaled, ghosted and hidden, `touching edge`, `if on edge, bounce`, `touching color` including the 5/5/4-bit tolerance). The golden values in `conformance/render/expected.json` were recorded from scratch-vm 5.0.300 + scratch-render 2.2.84 running with WebGL in a browser (`conformance/out/rendertest/index.html`); `node conformance/render/compare.js` checks scrust against them: 78/78 values identical, positions bit for bit.

Getting there required reproducing three things that are easy to miss:

- scratch-render stores positions, scales and matrices in `Float32Array`s. Bounds, and therefore positions after `if on edge, bounce`, are float32 values (`218.36251831054688`). `Xf` rounds through `f32` at the same places.
- Silhouettes are sampled at `floor(u * (width - 1))`, without rejecting coordinates outside `[0, 1]`, and rotated/scaled SVG skins use a 2x2 "linear" test. A sprite therefore extends about one texel beyond its quad.
- Collision scan regions are AABBs snapped outwards to integers, and `sin(180 deg)` is `1.22e-16` rather than 0, so an edge at an exact integer lands just past it and the region grows by a pixel. Together with the previous point this is why two sprites one pixel apart are "touching" in Scratch.

**Still approximate.** Scan regions always use the quad's AABB (scratch-render switches to tighter hull bounds for drawables whose hull has been computed). Silhouettes are rasterised at scale 1 (scratch-render keeps the largest MIP it has drawn). SVG size is the document size (scratch measures the rendered bounding box for Scratch 2 era files). Text bubbles use approximate metrics and the system (or embedded Noto Sans) font instead of Scratch's fonts and are drawn above all sprites; pen lines are anti-aliased by tiny-skia. No pixel comparison of the rendered frame itself has been made.

## Metal (macOS)

**Decision.** On macOS, drawing moves to the GPU through Metal (`crates/scrust-render/src/gpu.rs`, bindings from `objc2-metal`): sprites with all graphic effects, the stage, the pen layer (lines and stamps are rendered into a GPU texture) and the text overlays (bubbles, monitors, the ask box, rasterised on the CPU as before and uploaded as textures). Both the player (`SoftRenderer`) and the GUI's editor scene (`Scene`) use it. Without a Metal device, on other platforms, on wasm, with `SCRUST_RENDERER=cpu`, and in `--deterministic` runs (unless `SCRUST_RENDERER=metal`), the CPU compositor draws as before.

**What stays on the CPU.** Collision queries keep sampling the CPU silhouettes, as scratch-render does (§13), so `touching` results do not depend on the GPU. `touching color` needs the pen layer: it is read back from the GPU when it changed since the last read.

**How it matches the CPU path.** The vertex shader applies the same model transform (`Xf`, already rounded through f32), the fragment shader ports `Effects::point` and `Effects::color`, the same SVG MIP level is uploaded, sampling is nearest for distorted or exactly 1:1 drawables and bilinear (transparent outside the texture) otherwise, and blending is premultiplied "over" in gamma space. Pen lines are round-capped segments anti-aliased by distance, with the CPU path's half-pixel offset for 1 and 3 px pens.

**Measured.** The demo project rendered at 960x720 after 90 deterministic frames: 98.8 % of the pixels identical to the CPU frame, the rest anti-aliasing differences of at most 32/255, same game state. `conformance/render`: 78/78 with either backend. A pen test (lines of 1-9 px, a stamp, a `touching color` scan of the stage) found 194 of 195 hits in common; the two differing probes sit on anti-aliased line edges. Speed (editor scene, 960x720, rotated SVG sprites with effects): 20 sprites 89.6 ms -> 1.2 ms per frame, 200 sprites 661 ms -> 3.1 ms.

**Not done.** Frames are read back to memory (the native window still presents through softbuffer, the embedded host sends RGBA to the GUI) instead of being presented from a `CAMetalLayer`.
