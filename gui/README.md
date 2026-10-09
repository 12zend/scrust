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

## Layout

| Path | Role |
|---|---|
| `packages/scrust-vm` | what the GUI imports as `scratch-vm`: scratch-vm's editor model with execution disabled; green flag → compile → run, input forwarding, state sync |
| `packages/scrust-render` | what the GUI imports as `scratch-render`: the editor's stage drawn by `scrust_render::Scene` through the addon |
| `electron/main.js` | serves `build/`, runs `scrust check` / `scrust build`, starts the program with `--embed` and forwards its frames |
| `electron/preload.js` | loads the addon, exposes `scrustNative` and `scrustCompiler` to the page |
| `webpack.config.js` | scratch-gui's editor build with the two aliases |

Environment: `SCRUST_BIN` (CLI path), `SCRUST_NODE_ADDON` (addon path), `SCRUST_GUI_DEBUG=1` (build projects without optimisations).
