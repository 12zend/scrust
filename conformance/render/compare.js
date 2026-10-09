// Compare scrust's renderer-dependent behaviour with the golden values that
// were recorded from scratch-vm 5.0.300 + scratch-render 2.2.84 (WebGL) by
// opening ../out/rendertest/index.html in a browser (see README).
//   node compare.js
const fs = require('fs');
const path = require('path');
const {execFileSync, spawnSync} = require('child_process');
const ROOT = path.join(__dirname, '..', '..');
execFileSync('node', [path.join(__dirname, 'make.js')]);
const sb3 = path.join(ROOT, 'conformance', 'out', 'rendertest', 'project.sb3');
const outDir = path.join(ROOT, 'conformance', 'out', 'rendertest', 'bin');
execFileSync(path.join(ROOT, 'target/debug/scrust'), ['build', sb3, '-o', outDir], {stdio: ['ignore', 'ignore', 'inherit']});
const r = spawnSync(path.join(outDir, 'project'), ['--deterministic', '--trace', '3'], {encoding: 'utf8', maxBuffer: 1 << 26});
const last = JSON.parse(r.stdout.trim().split('\n').pop());
const got = last.targets[0].lists.find(l => l[0] === 'out')[1].map(v => String(v.v));
const exp = JSON.parse(fs.readFileSync(path.join(__dirname, 'expected.json'), 'utf8'));
let bad = 0;
const near = (a, b) => {
    // Positions that depend on rasterised bounds may differ by a fraction of a pixel.
    const pa = a.split(/[@, ]/).map(Number); const pb = b.split(/[@, ]/).map(Number);
    return pa.length === pb.length && pa.every((x, i) => (Number.isNaN(x) ? a === b : Math.abs(x - pb[i]) < 1e-6));
};
for (let i = 0; i < Math.max(got.length, exp.length); i++) {
    if (got[i] === exp[i]) continue;
    const tag = (got[i] !== undefined && exp[i] !== undefined && near(got[i], exp[i])) ? 'close' : 'DIFF ';
    if (tag === 'DIFF ') bad++;
    console.log(`${tag} #${i}: expected ${exp[i]}  got ${got[i]}`);
}
console.log(`${exp.length - bad}/${exp.length} values match`);
process.exit(bad ? 1 : 0);
