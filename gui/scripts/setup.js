// Install the dependencies of the vendored TurboWarp/scratch-gui and build
// everything (see build.js).
//
// vendor/scratch-gui is checked into this repository as plain files, taken from
// https://github.com/TurboWarp/scratch-gui at be382abea6af88db188162ca9a3e4e06ec4ca3b2.

const {execFileSync} = require('child_process');
const fs = require('fs');
const path = require('path');

const GUI_DIR = path.resolve(__dirname, '..');
const VENDOR = path.join(GUI_DIR, 'vendor', 'scratch-gui');

const sh = (cmd, args, options) => {
    console.log(`$ ${cmd} ${args.join(' ')}`);
    execFileSync(cmd, args, Object.assign({stdio: 'inherit'}, options));
};

if (!fs.existsSync(path.join(VENDOR, 'node_modules'))) {
    sh('npm', ['ci', '--no-audit', '--no-fund'], {cwd: VENDOR});
}

require('./build.js');
