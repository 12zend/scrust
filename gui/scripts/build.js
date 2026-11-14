// Build what the GUI loads.
//
//   node scripts/build.js                  Electron app: the scrust CLI (the
//                                          compiler the green flag runs), the
//                                          renderer addon (crates/scrust-node)
//                                          and the GUI bundle in build/
//   node scripts/build.js --gui-only       only the Electron app's webpack bundle
//   node scripts/build.js --web            browser version in build-web/ (static
//                                          files): the bundle, the scene module
//                                          (crates/scrust-scene-wasm) and the
//                                          interpreter (crates/scrust-interp),
//                                          both built for wasm32
//   node scripts/build.js --web --gui-only only the browser version's bundle

const {execFileSync} = require('child_process');
const fs = require('fs');
const path = require('path');

const GUI_DIR = path.resolve(__dirname, '..');
const REPO = path.resolve(GUI_DIR, '..');
const VENDOR = path.join(GUI_DIR, 'vendor', 'scratch-gui');
const WEB = process.argv.includes('--web');
const GUI_ONLY = process.argv.includes('--gui-only');

const sh = (cmd, args, options) => {
    console.log(`$ ${cmd} ${args.join(' ')}`);
    execFileSync(cmd, args, Object.assign({stdio: 'inherit'}, options));
};

const WASM_OUT = path.join(REPO, 'target', 'wasm32-unknown-unknown', 'release');

if (!GUI_ONLY) {
    if (WEB) {
        sh('cargo', ['build', '--release', '-p', 'scrust-scene-wasm', '-p', 'scrust-interp', '--target', 'wasm32-unknown-unknown'], {cwd: REPO});
    } else {
        sh('cargo', ['build', '--release', '-p', 'scrust-cli', '-p', 'scrust-node'], {cwd: REPO});
    }
}

sh(process.execPath, [path.join(VENDOR, 'node_modules', 'webpack', 'bin', 'webpack.js'), '--config', path.join(GUI_DIR, 'webpack.config.js')], {
    cwd: VENDOR,
    env: Object.assign({}, process.env, {
        // webpack 4 hashes with md4, which OpenSSL 3 only offers as a legacy algorithm.
        NODE_OPTIONS: `${process.env.NODE_OPTIONS || ''} --openssl-legacy-provider`.trim(),
        CI: 'true',
        SCRUST_GUI_TARGET: WEB ? 'web' : ''
    })
});

if (WEB) {
    fs.copyFileSync(path.join(WASM_OUT, 'scrust_scene_wasm.wasm'), path.join(GUI_DIR, 'build-web', 'scrust-scene.wasm'));
    fs.copyFileSync(path.join(WASM_OUT, 'scrust_interp.wasm'), path.join(GUI_DIR, 'build-web', 'scrust-interp.wasm'));
    // The site root opens the editor too (the GUI then shows editor.html).
    fs.copyFileSync(path.join(GUI_DIR, 'build-web', 'editor.html'), path.join(GUI_DIR, 'build-web', 'index.html'));
    console.log(`built ${path.join(GUI_DIR, 'build-web')}`);
}
