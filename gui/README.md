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

The same editor without Electron: the stage is drawn by `scrust_render::Scene` built for wasm32 (`crates/scrust-scene-wasm`), and the green flag sends the project to a compile server that returns the program built for wasm32, which then runs inside the page. Rust cannot run in a browser, so compiling still needs a machine with the toolchain; by default that is a local process on your own machine (127.0.0.1).

```bash
rustup target add wasm32-unknown-unknown
```

```bash
npm run build:web
```

```bash
npm run serve:web
```

Then open http://127.0.0.1:8080. `build:web` writes the static page to `build-web/` (it can be hosted anywhere, e.g. Cloudflare Pages); built with `SCRUST_COMPILER_URL=https://host` it compiles on that server instead of the one that served the page. `web/server.js` lists its environment variables (`PORT`, `HOST`, `SCRUST_CORS_ORIGIN`, `SCRUST_STATIC`).

## Layout

| Path | Role |
|---|---|
| `packages/scrust-vm` | what the GUI imports as `scratch-vm`: scratch-vm's editor model with execution disabled; green flag → compile → run, input forwarding, state sync |
| `packages/scrust-render` | what the GUI imports as `scratch-render`: the editor's stage drawn by `scrust_render::Scene` through the addon |
| `electron/main.js` | serves `build/`, runs `scrust check` / `scrust build`, starts the program with `--embed` and forwards its frames |
| `electron/preload.js` | loads the addon, exposes `scrustNative` and `scrustCompiler` to the page |
| `web/bridge.js` | browser versions of what the preload provides: the scene module and a compiler/runner for wasm programs |
| `web/entry.js` | entry of the browser build (`SCRUST_GUI_TARGET=web`) |
| `web/server.js` | compile server (`POST /api/compile`, sb3 in, wasm out) and static server for `build-web/` |
| `webpack.config.js` | scratch-gui's editor build with the two aliases |

Environment: `SCRUST_BIN` (CLI path), `SCRUST_NODE_ADDON` (addon path), `SCRUST_GUI_DEBUG=1` (build projects without optimisations).
