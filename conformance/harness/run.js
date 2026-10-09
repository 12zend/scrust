// Differential test runner: scratch-vm (ground truth) vs scrust.
//   node run.js [--only substr] [--mode both|nounbox|unbox] [--release] [--keep-going] [--jobs N]
// Test cases live in ../tests/*.js; each module exports an array of
//   {name, project (Project) | json, frames, seed?, fps?, maxPasses?, turbo?, answers?}
const fs = require('fs');
const path = require('path');
const {execFile, execFileSync} = require('child_process');

const ROOT = path.join(__dirname, '..', '..');
const OUT = path.join(ROOT, 'conformance', 'out');
const SCRUST = path.join(ROOT, 'target', 'debug', 'scrust');

const argv = process.argv.slice(2);
const opt = {only: null, mode: 'both', release: false, jobs: 4, verbose: false};
for (let i = 0; i < argv.length; i++) {
    if (argv[i] === '--only') opt.only = argv[++i];
    else if (argv[i] === '--mode') opt.mode = argv[++i];
    else if (argv[i] === '--release') opt.release = true;
    else if (argv[i] === '--jobs') opt.jobs = Number(argv[++i]);
    else if (argv[i] === '-v') opt.verbose = true;
}

const run = (cmd, args) => new Promise(resolve => {
    execFile(cmd, args, {maxBuffer: 1 << 30}, (err, stdout, stderr) => resolve({err, stdout, stderr}));
});

// A canonical numeric string is observably equal to its number once a
// variable is unboxed (PRODUCT.md §8.6): normalise both sides.
const canonical = s => /^-?\d+(\.\d+)?$/.test(s) && String(Number(s)) === s;
function normalise (line) {
    let o;
    try { o = JSON.parse(line); } catch (e) { return line; }
    const fix = v => (v.t === 'str' && canonical(v.v) ? {t: 'num', v: v.v} : v);
    for (const t of o.targets) {
        t.vars = t.vars.map(([n, v]) => [n, fix(v)]);
        t.lists = t.lists.map(([n, items]) => [n, items.map(fix)]);
    }
    return JSON.stringify(o);
}

function firstDiff (exp, got) {
    const n = Math.max(exp.length, got.length);
    for (let i = 0; i < n; i++) {
        if (exp[i] !== got[i]) {
            let detail = '';
            try {
                const a = JSON.parse(exp[i] || '{}');
                const b = JSON.parse(got[i] || '{}');
                for (const k of ['threads', 'passes', 'rng']) if (a[k] !== b[k]) detail += `\n      ${k}: expected ${a[k]} got ${b[k]}`;
                const m = Math.max((a.targets || []).length, (b.targets || []).length);
                const short = v => { const s = JSON.stringify(v); return s === undefined ? 'undefined' : (s.length > 300 ? s.slice(0, 300) + '…' : s); };
                let shown = 0;
                for (let j = 0; j < m && shown < 8; j++) {
                    const x = (a.targets || [])[j] || {};
                    const y = (b.targets || [])[j] || {};
                    if (JSON.stringify(x) === JSON.stringify(y)) continue;
                    const who = `target[${j}] ${x.name || y.name}${(x.clone || y.clone) ? ' (clone)' : ''}`;
                    for (const k of ['name', 'clone', 'x', 'y', 'dir', 'size', 'costume', 'visible', 'say']) {
                        if (JSON.stringify(x[k]) !== JSON.stringify(y[k])) { detail += `\n      ${who} ${k}: expected ${short(x[k])} got ${short(y[k])}`; shown++; }
                    }
                    for (const kind of ['vars', 'lists']) {
                        const xs = x[kind] || []; const ys = y[kind] || [];
                        for (let q = 0; q < Math.max(xs.length, ys.length) && shown < 8; q++) {
                            const [xn, xv] = xs[q] || []; const [yn, yv] = ys[q] || [];
                            if (JSON.stringify(xs[q]) === JSON.stringify(ys[q])) continue;
                            if (kind === 'lists' && xn === yn && Array.isArray(xv) && Array.isArray(yv)) {
                                if (xv.length !== yv.length) { detail += `\n      ${who} list ${xn}: length expected ${xv.length} got ${yv.length}`; shown++; }
                                for (let r = 0; r < Math.max(xv.length, yv.length) && shown < 8; r++) {
                                    if (JSON.stringify(xv[r]) !== JSON.stringify(yv[r])) { detail += `\n      ${who} list ${xn}[${r + 1}]: expected ${short(xv[r])} got ${short(yv[r])}`; shown++; }
                                }
                            } else { detail += `\n      ${who} ${kind}[${q}]: expected ${short(xs[q])} got ${short(ys[q])}`; shown++; }
                        }
                    }
                }
            } catch (e) { detail = `\n      expected ${String(exp[i]).slice(0, 300)}\n      got      ${String(got[i]).slice(0, 300)}`; }
            return `frame ${i + 1}:${detail}`;
        }
    }
    return null;
}

async function runCase (c) {
    const file = path.join(OUT, `${c.name}.json`);
    fs.writeFileSync(file, c.json || c.project.json());
    const common = ['--frames', String(c.frames || 30), '--seed', String(c.seed || 1), '--fps', String(c.fps || 30), '--max-passes', String(c.maxPasses || 1)];
    if (c.turbo) common.push('--turbo');
    if (c.answers) common.push('--answers', c.answers.join('|'));
    const ref = await run('node', [path.join(__dirname, 'trace.js'), file, ...common]);
    if (ref.err) return {name: c.name, ok: false, msg: `scratch-vm tracer failed: ${ref.stderr.slice(0, 2000)}`};
    const exp = ref.stdout.trim().split('\n');
    const results = [];
    const modes = opt.mode === 'both' ? ['nounbox', 'unbox'] : [opt.mode];
    for (const mode of modes) {
        const args = ['trace', file, ...common, '--allow-unsupported'];
        if (mode === 'nounbox') args.push('--no-unbox');
        if (!opt.release) args.push('--debug');
        const got = await run(SCRUST, args);
        if (got.err) { results.push(`[${mode}] scrust failed: ${(got.stderr || String(got.err)).slice(-1500)}`); continue; }
        let g = got.stdout.trim().split('\n');
        let e = exp;
        if (mode === 'unbox') { g = g.map(normalise); e = e.map(normalise); }
        const d = firstDiff(e, g);
        if (d) results.push(`[${mode}] mismatch at ${d}`);
    }
    return {name: c.name, ok: results.length === 0, msg: results.join('\n    ')};
}

async function main () {
    fs.mkdirSync(OUT, {recursive: true});
    execFileSync('cargo', ['build', '-q', '-p', 'scrust-cli'], {cwd: ROOT, stdio: 'inherit'});
    let cases = [];
    const dir = path.join(__dirname, '..', 'tests');
    for (const f of fs.readdirSync(dir).filter(f => f.endsWith('.js')).sort()) {
        const mod = require(path.join(dir, f));
        const list = typeof mod === 'function' ? mod() : mod;
        for (const c of list) cases.push(Object.assign({}, c, {name: `${path.basename(f, '.js')}__${c.name}`}));
    }
    if (opt.only) cases = cases.filter(c => c.name.includes(opt.only));
    let failed = 0;
    let idx = 0;
    const worker = async () => {
        while (idx < cases.length) {
            const c = cases[idx++];
            const r = await runCase(c);
            if (r.ok) { if (opt.verbose) console.log(`ok    ${r.name}`); } else { failed++; console.log(`FAIL  ${r.name}\n    ${r.msg}`); }
        }
    };
    await Promise.all(Array.from({length: opt.jobs}, worker));
    console.log(`\n${cases.length - failed}/${cases.length} cases passed`);
    process.exit(failed ? 1 : 0);
}
main();
