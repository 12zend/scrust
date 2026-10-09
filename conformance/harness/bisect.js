// Find the first fuzz statement at which scrust and scratch-vm diverge.
//   node bisect.js <seed> [--unbox]
const fs = require('fs');
const path = require('path');
const {execFileSync} = require('child_process');
const {build} = require('../tests/03_fuzz');
const ROOT = path.join(__dirname, '..', '..');
const seed = Number(process.argv[2]);
const unbox = process.argv.includes('--unbox');
const file = path.join(ROOT, 'conformance', 'out', `bisect_${seed}.json`);
const run = (cmd, args) => execFileSync(cmd, args, {maxBuffer: 1 << 28}).toString();
function same (n) {
    const p = build(seed, n);
    fs.writeFileSync(file, p.json());
    const common = ['--frames', '2', '--seed', String(seed), '--max-passes', '1'];
    const exp = run('node', [path.join(__dirname, 'trace.js'), file, ...common]);
    const got = run(path.join(ROOT, 'target/debug/scrust'), ['trace', file, ...common, '--debug', '--allow-unsupported', ...(unbox ? [] : ['--no-unbox'])]);
    return exp === got;
}
let lo = 0; let hi = 120;
if (same(hi)) { console.log('no divergence'); process.exit(0); }
while (hi - lo > 1) { const mid = (lo + hi) >> 1; if (same(mid)) lo = mid; else hi = mid; }
const proj = JSON.parse(build(seed, hi).json());
const blocks = proj.targets[1].blocks;
// Walk the script to statement #hi and pretty-print it.
let id = Object.keys(blocks).find(k => blocks[k].opcode === 'event_whenflagclicked');
id = blocks[id].next;
for (let i = 1; i < hi; i++) id = blocks[id].next;
const show = bid => {
    const blk = blocks[bid];
    const ins = Object.entries(blk.inputs).map(([k, v]) => `${k}=${typeof v[1] === 'string' ? show(v[1]) : JSON.stringify(v[1])}`);
    const fs2 = Object.entries(blk.fields).map(([k, v]) => `${k}:${JSON.stringify(v[0])}`);
    return `${blk.opcode.replace(/^(operator|data|control)_/, '')}(${[...fs2, ...ins].join(', ')})`;
};
console.log(`first divergence at statement ${hi}:\n  ${show(id)}`);
