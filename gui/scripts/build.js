// Build everything the Electron app loads: the scrust CLI (the compiler the
// green flag runs), the renderer addon (crates/scrust-node) and the GUI bundle.
//
//   node scripts/build.js            all three
//   node scripts/build.js --gui-only only the webpack bundle

const {execFileSync} = require('child_process');
const path = require('path');

const GUI_DIR = path.resolve(__dirname, '..');
const REPO = path.resolve(GUI_DIR, '..');
const VENDOR = path.join(GUI_DIR, 'vendor', 'scratch-gui');

const sh = (cmd, args, options) => {
    console.log(`$ ${cmd} ${args.join(' ')}`);
    execFileSync(cmd, args, Object.assign({stdio: 'inherit'}, options));
};

if (!process.argv.includes('--gui-only')) {
    sh('cargo', ['build', '--release', '-p', 'scrust-cli', '-p', 'scrust-node'], {cwd: REPO});
}

sh(process.execPath, [path.join(VENDOR, 'node_modules', 'webpack', 'bin', 'webpack.js'), '--config', path.join(GUI_DIR, 'webpack.config.js')], {
    cwd: VENDOR,
    env: Object.assign({}, process.env, {
        // webpack 4 hashes with md4, which OpenSSL 3 only offers as a legacy algorithm.
        NODE_OPTIONS: `${process.env.NODE_OPTIONS || ''} --openssl-legacy-provider`.trim(),
        CI: 'true'
    })
});
