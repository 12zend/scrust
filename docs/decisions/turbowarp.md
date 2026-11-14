# Reference behaviour: TurboWarp

**Decision.** scrust reproduces what a project does in normal TurboWarp (TurboWarp/scratch-vm with its compiler, TurboWarp/scratch-render, default settings: 30 fps, no warp timer, fencing and misc limits on), not vanilla scratch-vm. It stays an ahead-of-time compiler: no TurboWarp code is embedded; where the two engines differ, the generated code and the runtime follow TurboWarp.

**How it is checked.** `conformance/twref` runs a TurboWarp scratch-vm checkout (`SCRUST_REF_VM`) with TurboWarp's scratch-render in a browser, driven deterministically like `harness/trace.js` (seeded `Math.random`, virtual clock, one scheduler pass per frame), and saves trace lines and stage screenshots. `node conformance/twref/run.js scrust|compare` produces the same from scrust and compares them (see conformance/README.md).

**Differences from vanilla that scrust now follows.**

| Area | TurboWarp | Where |
|---|---|---|
| `Cast.compare` "not actually zero" | a string without digits and without tab is NaN (so `"\t"` is 0) | `value::compare`, `Ty::of_value` |
| `sin` / `cos` / `tan` rounding | `Math.round(x * 1e10) / 1e10` | `ops::sin_deg` etc. |
| `data_listcontents` | no separator iff every item's *string form* has length 1 | `ops::list_contents`, `Elem::is_single_char` |
| `pick random a to a` with equal literal inputs | folded to the constant, no random number drawn | codegen `op` |
| `and` / `or` | short-circuit (compiled to `&&` / `||`) | codegen |
| Evaluation order of inputs | argument order of the generated JavaScript (`letter of`: string first), not project.json order | `lower::eval_order`, `binop` |
| Unbound boolean argument `is compiled?` / `is TurboWarp?` | `true` | `lower_reporter` |
| `set/change sound effect`, `set/change volume` | return `Promise.resolve()`: one-tick wait, and the loop-end yield is skipped in tail position | `Cmd::is_promise` |
| Math functions | current V8: correctly rounded, `atan` as LLVM libc | [math-libm.md](math-libm.md) |
| Inputs of `repeat`, `wait`, `glide`, `broadcast and wait`, ... | evaluated once, not on every re-execution | [reexecution.md](reexecution.md) |
| Calls to a procedure with an empty or missing definition | compiled to nothing; arguments not evaluated | `lower_stmt` |
| Recursion yield | only before a direct, non-warp self-call | [recursion-yield.md](recursion-yield.md) |
| List length | no 200,000 item limit | `List::add`, `List::insert` |
| Variables a script refers to but the project does not declare | created when the script is first compiled (its first start), on that target | `ScriptDef::creates`, trace |

**Project settings.** TurboWarp reads a stage comment ending in ` // _twconfig_` (`Runtime.parseProjectOptions`). scrust reads it at build time (`Project::tw_config`): `framerate` (capped at 250, 0 = 60) becomes the default `--fps`, `turbo` the default turbo mode, and `runtimeOptions` become runtime options: `fencing: false` disables fencing, the `set size` clamp and the stage clamp of collision regions (`offscreenTouching`); `miscLimits: false` removes the pen size clamp (also with `hq`), widens the pitch range to ±1000, drops the one-tick wait of sound effect/volume blocks and gives mouse coordinates three decimals; `maxClones` replaces the 300 clone limit; `hq` turns on high quality pen; `width` / `height` set the stage size (fencing, edges, mouse coordinates, the window and screenshots use it). `interpolation` is not implemented.

**scratch-render details reproduced.** Unless the project uses high quality pen (`hq`), `Drawable.updatePosition` rounds positions to whole pixels (`Math.round`): drawing, stamping, collisions and fencing all use the rounded position while the sprite keeps its exact x/y. Bitmap costumes are always sampled NEAREST; SVG costumes only when undistorted, upright (direction a multiple of 90) and drawn at about 100%, otherwise LINEAR. `touching color` composites candidates front to back into a `Uint8ClampedArray`, rounding (half to even) after every effect operation and every layer, samples the pen layer at `floor(u * (width - 1))` like any skin, and ignores shape-changing effects. Monitors and the ask prompt are HTML in the GUI; built programs accept `--no-monitors` for screenshots compared with the canvas.

**SVG costumes.** The size is the `viewBox`, parsed the way Blink parses SVG numbers (digit by digit in float32, so `0.13503` is 0.13502998650074005, not the nearest float). Without a viewBox, scratch-svg-renderer's `transformMeasurements` applies: the content's `getBBox()` (computed from the leaves; usvg's own image bounding box applies the image transform twice), enlarged by half of `findLargestStrokeWidth` (1 for any element with a `stroke` attribute, even `none`, or the largest `stroke-width` attribute). MIP canvases truncate `size * scale`. Embedded images are drawn pixelated, picking the texel Skia picks at exact boundaries. When the image is drawn smaller than half size, Chrome samples a reduced copy: level `n` (the smallest with `2^-n` at least the drawn scale), `ceil(size / 2)` per level, made by nearest sampling at pixel centres (ties to the lower pixel); `skin::image_level`, `downsample_level`.

**Colour profiles.** Chrome converts images with an embedded ICC profile (PNG `iCCP`, JPEG `ICC_PROFILE`) to the sRGB canvas, for bitmap costumes and images inside SVGs alike: Skia's `SkColorSpace` snapping (curves within 0.001 of sRGB, 2.2 or linear, gamuts within 0.01 of sRGB), the gamut matrix from skcms, and an encode with skcms's own inverse of the sRGB curve (its constants come from an approximate `powf`, which is why textbook math rounds about 4 % of colours differently). `color.rs`; on a 65,536-colour Display P3 grid it matches Chrome on all but one colour.

**Pen lines.** The Metal pen shader is scratch-render's line shader (`DRAW_MODE_line`: the `expandedRadius` quad, the round-cap distance formula, premultiplied colour, attributes rounded to float32 as `PenSkin._drawLineOnBuffer` stores them), so anti-aliased pen pixels are identical.

**Stamps and texels.** Stamps and sprite draws use scratch-render's vertex math, viewport and triangle layout, and ANGLE's framebuffer orientation, so the GPU picks the same texels and edge pixels as the browser (see rendering.md, "Geometry as WebGL draws it"); image scanners that read stamps back with `touching color` depend on it.

**Silhouettes.** An SVG skin's silhouette is its largest MIP so far (`isLargestMIP`), unless that MIP's canvas is empty. MIPs are made by drawing (every frame for visible drawables, also when the frame is not shown: `Renderer::frame_drawn`), by stamping, by `updateSilhouette(drawable.scale)` before CPU queries, and at 100% by `_touchingBounds` before every `touching` query. A hidden drawable has no convex hull, so its bounds (`touching edge`, bouncing) are the quad's AABB.

**Speech bubbles.** `TextBubbleSkin` is ported: the same line wrapping (measured with the bubble font, Helvetica on macOS), padding, tail and corner geometry, and placement (`_positionBubble`: left or right of the sprite, flipped when it would leave the stage, clamped to the stage).

**Extensions.** `translate_getViewerLanguage` reports `English` (the reference's `en` locale); `translate_getTranslate` returns its text unchanged (no translation server). Music: `play note / drum for beats` and `rest for beats` wait like TurboWarp's stack timer (beats clamped to 0..100, a note for 0 beats does not wait), tempo blocks work (20..500), instruments are accepted; no notes are synthesised.

**Engine details that both share but scrust had missed.** One monitor thread per visible monitor every frame (it only shows in the trace's pass count); SVG size taken from `viewBox` read as float32, the rotation centre shifted by the viewBox origin, SVGs without a viewBox cropped to their content's bounding box, and zero-sized SVGs as empty skins whose size stays 0 (so `set size` is unclamped and the drawable's matrix becomes NaN, which disables fencing: projects use this to place sprites anywhere).

**Comparison.** Lists longer than `hashOver` (page) / `SCRUST_HASH_OVER` (run.js) are traced as a length and FNV-1a hash of their normalised items, so engines with very large lists fit in the browser tab.

**Not reproduced.** Interpolation from the project config. Text anti-aliasing differs slightly from the browser (resvg is not Skia), image rectangle edges are anti-aliased by supersampling rather than analytically, JPEG decoding is not libjpeg-turbo's, and system fonts other than the bundled ones may be substituted.
