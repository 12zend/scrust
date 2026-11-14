# scrust GUI

TurboWarp's [scratch-gui](https://github.com/TurboWarp/scratch-gui) in Electron, with `scratch-vm` and `scratch-render` replaced by scrust: the green flag compiles the project to Rust and runs it as a native program whose frames are drawn on the stage. Design and limitations: [docs/decisions/gui.md](../docs/decisions/gui.md).

## Setup

Needs Node, git and the Rust toolchain. From this directory:

```bash
npm install
```

```bash
npm run setup
```

`setup` installs the dependencies of the vendored scratch-gui (`vendor/scratch-gui`, checked in; upstream commit noted in `scripts/setup.js`) and runs `build` (the `scrust` CLI, the renderer addon `crates/scrust-node`, and the webpack bundle in `build/`).

```bash
npm start
```

After changing Rust code run `npm run build`; after changing only `packages/` or `src/`, `node scripts/build.js --gui-only`.

## Browser version

The same editor as static files, without Electron and without any server-side processing: the stage is drawn by `scrust_render::Scene` built for wasm32 (`crates/scrust-scene-wasm`), and the green flag runs the project in the page with scrust's interpreter (`crates/scrust-interp`, also wasm32), which loads the .sb3 itself. rustc cannot run in a browser, so this build interprets the IR instead of compiling it; it behaves the same (see [docs/decisions/interpreter.md](../docs/decisions/interpreter.md)) but runs slower than the Electron app's native programs.

```bash
rustup target add wasm32-unknown-unknown
```

```bash
npm run build:web
```

```bash
npm run serve:web
```

Then open http://127.0.0.1:8080. `serve:web` is only a static file server for trying it locally: `build-web/` can be put on any static host (Cloudflare Pages, GitHub Pages, ...) as it is.

## Layout

| Path | Role |
|---|---|
| `packages/scrust-vm` | what the GUI imports as `scratch-vm`: scratch-vm's editor model with execution disabled; green flag → compile → run, input forwarding, state sync |
| `packages/scrust-render` | what the GUI imports as `scratch-render`: the editor's stage drawn by `scrust_render::Scene` through the addon |
| `electron/main.js` | serves `build/`, runs `scrust check` / `scrust build`, starts the program with `--embed` and forwards its frames |
| `electron/preload.js` | loads the addon, exposes `scrustNative` and `scrustCompiler` to the page |
| `web/bridge.js` | browser versions of what the preload provides: the scene module, and the interpreter that loads and runs the project in the page |
| `web/entry.js` | entry of the browser build (`SCRUST_GUI_TARGET=web`) |
| `web/server.js` | static file server for `build-web/` (local testing only) |
| `webpack.config.js` | scratch-gui's editor build with the two aliases |

Environment: `SCRUST_BIN` (CLI path), `SCRUST_NODE_ADDON` (addon path), `SCRUST_GUI_DEBUG=1` (build projects without optimisations).
