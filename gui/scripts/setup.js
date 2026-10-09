// Fetch TurboWarp/scratch-gui at the pinned commit into vendor/, install its
// dependencies and build everything (see build.js).

const {execFileSync} = require('child_process');
const fs = require('fs');
const path = require('path');

const GUI_REPO = 'https://github.com/TurboWarp/scratch-gui.git';
const GUI_COMMIT = 'be382abea6af88db188162ca9a3e4e06ec4ca3b2';

const GUI_DIR = path.resolve(__dirname, '..');
const VENDOR = path.join(GUI_DIR, 'vendor', 'scratch-gui');

const sh = (cmd, args, options) => {
    console.log(`$ ${cmd} ${args.join(' ')}`);
    execFileSync(cmd, args, Object.assign({stdio: 'inherit'}, options));
};

if (!fs.existsSync(path.join(VENDOR, '.git'))) {
    fs.mkdirSync(VENDOR, {recursive: true});
    sh('git', ['init', '-q'], {cwd: VENDOR});
    sh('git', ['remote', 'add', 'origin', GUI_REPO], {cwd: VENDOR});
}
const head = execFileSync('git', ['rev-parse', '--verify', '-q', 'HEAD'], {cwd: VENDOR, encoding: 'utf8', stdio: ['ignore', 'pipe', 'ignore']})
    .trim();
if (head !== GUI_COMMIT) {
    sh('git', ['fetch', '--depth', '1', 'origin', GUI_COMMIT], {cwd: VENDOR});
    sh('git', ['checkout', '-q', '--detach', GUI_COMMIT], {cwd: VENDOR});
}
if (!fs.existsSync(path.join(VENDOR, 'node_modules'))) {
    sh('npm', ['ci', '--no-audit', '--no-fund'], {cwd: VENDOR});
}

require('./build.js');
