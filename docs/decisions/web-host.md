# Web target without wasm-bindgen

`scrust build --target wasm` generates a `cdylib` for `wasm32-unknown-unknown`. The page (`crates/scrust-player/web/index.html`) talks to it through plain exports (`scrust_init`, `scrust_frame`, `scrust_key`, `scrust_mouse_*`, `scrust_sound_*`) and three imports for Web Audio. The frame is an RGBA buffer in linear memory that the page blits with `putImageData`.

Reasons: no extra tooling (wasm-bindgen CLI versions must match the crate), the same CPU renderer as native, and a single self-contained file with `--single-html` (the module is embedded as base64). Sounds are decoded by the browser (`decodeAudioData`) from the embedded asset bytes.

Fonts are compiled into every build (see `fonts.md`). On the web there are no system fonts, so characters outside the embedded fonts' coverage (e.g. CJK) are not drawn there.

The wasm standard library comes from a rustup toolchain; when rustup is installed the CLI builds wasm targets with `rustup which cargo` / `rustc` even if another `cargo` is first on `PATH`.

The page fetches a sound's bytes (`scrust_sound_ptr` / `_len`) from inside `js_sound_play`, i.e. while `scrust_frame` holds the host state, so those two exports read the program through a separate thread-local instead of borrowing the state (borrowing it panicked on the first sound).
