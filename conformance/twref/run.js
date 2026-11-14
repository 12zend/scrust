// scrust side of the TurboWarp reference comparison, and the comparison.
//   node run.js scrust <dir|file.sb3>... [--frames N] [--trace-every K] [--every S] [--jobs J] [--only s] [--redo]
//   node run.js compare <dir|file.sb3>... [--only s] [--pixels]
// The reference side is the browser page (serve.js + entry.js) opened with
//   /?list=*&frames=N&traceEvery=K&every=S
// using the same N, K and S. Results:
//   conformance/out/twref/results/<base>.ref.jsonl, <base>-f<frame>.png  (TurboWarp)
//   conformance/out/twref/scrust/<base>.jsonl,      <base>-f<frame>.png  (scrust)
const fs = require('fs');
const path = require('path');
const readline = require('readline');
const {spawn, execFileSync, spawnSync} = require('child_process');

const ROOT = path.join(__dirname, '..', '..');
const OUT = path.join(ROOT, 'conformance', 'out', 'twref');
const REF = path.join(OUT, 'results');
const GOT = path.join(OUT, 'scrust');
const BUILD = path.join(OUT, 'build');
const SCRUST = path.join(ROOT, 'target', 'release', 'scrust');

const [mode, ...argv] = process.argv.slice(2);
const opt = {frames: 300, traceEvery: 30, every: 30, jobs: 3, only: null, redo: false, pixels: false, inputs: []};
for (let i = 0; i < argv.length; i++) {
    const a = argv[i];
    if (a === '--frames') opt.frames = Number(argv[++i]);
    else if (a === '--trace-every') opt.traceEvery = Number(argv[++i]);
    else if (a === '--every') opt.every = Number(argv[++i]);
    else if (a === '--jobs') opt.jobs = Number(argv[++i]);
    else if (a === '--only') opt.only = argv[++i];
    else if (a === '--redo') opt.redo = true;
    else if (a === '--pixels') opt.pixels = true;
    else opt.inputs.push(a);
}
const baseOf = f => path.basename(f).replace(/\.sb3$/, '').replace(/[^\w.-]+/g, '_');
function projects () {
    let files = [];
    for (const i of opt.inputs) {
        if (fs.statSync(i).isDirectory()) files.push(...fs.readdirSync(i).filter(f => f.endsWith('.sb3')).sort().map(f => path.join(i, f)));
        else files.push(i);
    }
    if (opt.only) files = files.filter(f => path.basename(f).includes(opt.only));
    return files;
}
const run = (cmd, args, outFile, env, timeoutS) => new Promise(resolve => {
    const out = outFile ? fs.openSync(outFile, 'w') : 'ignore';
    let stderr = '';
    const p = spawn(cmd, args, {stdio: ['ignore', out, 'pipe'], env: Object.assign({}, process.env, env || {})});
    p.stderr.on('data', d => { stderr = (stderr + d).slice(-4000); });
    const timer = setTimeout(() => { p.kill('SIGKILL'); stderr += '\n[timeout]'; }, (timeoutS || 1800) * 1000);
    p.on('close', code => { clearTimeout(timer); if (outFile) fs.closeSync(out); resolve({code, stderr}); });
});

async function scrustOne (file) {
    const base = baseOf(file);
    if (!opt.redo && fs.existsSync(path.join(GOT, `${base}.done`))) return `skip  ${base}`;
    fs.rmSync(path.join(GOT, `${base}.error`), {force: true});
    const dir = path.join(BUILD, base);
    const t0 = Date.now();
    const b = await run(SCRUST, ['build', file, '-o', dir, '--release', '--allow-unsupported'], null, {}, 1800);
    if (b.code !== 0) { fs.writeFileSync(path.join(GOT, `${base}.error`), b.stderr); return `BUILD ${base}\n    ${b.stderr.slice(-600)}`; }
    const exe = fs.readdirSync(dir).map(f => path.join(dir, f)).find(f => fs.statSync(f).isFile() && (fs.statSync(f).mode & 0o111));
    const r = await run(exe, ['--deterministic', '--trace', String(opt.frames), '--trace-every', String(opt.traceEvery), '--scale', '1',
        '--no-monitors', '--screenshot-every', String(opt.every), '--screenshot', path.join(GOT, `${base}-f%d.png`)], path.join(GOT, `${base}.jsonl`), {SCRUST_RENDERER: 'metal'}, 1800);
    if (r.code !== 0) { fs.writeFileSync(path.join(GOT, `${base}.error`), r.stderr); return `RUN   ${base}\n    ${r.stderr.slice(-600)}`; }
    fs.writeFileSync(path.join(GOT, `${base}.done`), `${opt.frames}`);
    return `ok    ${base} (${((Date.now() - t0) / 1000).toFixed(0)}s)`;
}

// ---------------------------------------------------------------- compare
// A string that is exactly the canonical form of its number ("2.22e-16",
// "Infinity") behaves like the number; TurboWarp stores such literals as
// numbers, scrust may keep either.
const canonical = s => s !== '' && String(Number(s)) === s && s !== 'NaN';
const HASH_OVER = Number(process.env.SCRUST_HASH_OVER || 0);
const hashItems = items => {
    let h = 0x811c9dc5;
    for (const it of items) {
        const str = JSON.stringify(it);
        for (let i = 0; i < str.length; i++) {
            h ^= str.charCodeAt(i);
            h = Math.imul(h, 0x01000193) >>> 0;
        }
    }
    return [{t: 'hash', v: `${items.length}:${h}`}];
};
function normalise (o) {
    const fix = v => (v.t === 'str' && canonical(v.v) ? {t: 'num', v: v.v} : v);
    for (const t of o.targets) {
        t.vars = t.vars.map(([n, v]) => [n, fix(v)]);
        t.lists = t.lists.map(([n, items]) => {
            if (items.length === 1 && items[0].t === 'hash') return [n, items];
            // Hash long lists from scrust the way the reference page did.
            if (HASH_OVER && items.length > HASH_OVER) return [n, hashItems(items.map(fix))];
            return [n, items.map(fix)];
        });
    }
    return o;
}
function describe (a, b) {
    const lines = [];
    for (const k of ['threads', 'passes', 'rng']) if (a[k] !== b[k]) lines.push(`${k}: expected ${a[k]} got ${b[k]}`);
    const m = Math.max(a.targets.length, b.targets.length);
    const short = v => { const s = JSON.stringify(v); return s === undefined ? 'undefined' : (s.length > 160 ? s.slice(0, 160) + '…' : s); };
    for (let j = 0; j < m && lines.length < 10; j++) {
        const x = a.targets[j] || {};
        const y = b.targets[j] || {};
        if (JSON.stringify(x) === JSON.stringify(y)) continue;
        const who = `[${j}] ${x.name || y.name}${(x.clone || y.clone) ? '(clone)' : ''}`;
        for (const k of ['name', 'clone', 'x', 'y', 'dir', 'size', 'costume', 'visible', 'say']) {
            if (JSON.stringify(x[k]) !== JSON.stringify(y[k])) lines.push(`${who} ${k}: expected ${short(x[k])} got ${short(y[k])}`);
        }
        for (const kind of ['vars', 'lists']) {
            const xs = x[kind] || []; const ys = y[kind] || [];
            for (let q = 0; q < Math.max(xs.length, ys.length) && lines.length < 10; q++) {
                if (JSON.stringify(xs[q]) === JSON.stringify(ys[q])) continue;
                const [xn, xv] = xs[q] || []; const [yn, yv] = ys[q] || [];
                if (kind === 'lists' && xn === yn && Array.isArray(xv) && Array.isArray(yv)) {
                    let n = 0; let first = -1;
                    for (let r = 0; r < Math.max(xv.length, yv.length); r++) if (JSON.stringify(xv[r]) !== JSON.stringify(yv[r])) { n++; if (first < 0) first = r; }
                    lines.push(`${who} list ${xn}: len ${xv.length}/${yv.length}, ${n} differ, first [${first + 1}] expected ${short(xv[first])} got ${short(yv[first])}`);
                } else lines.push(`${who} ${kind} ${xn || yn}: expected ${short(xv)} got ${short(yv)}`);
            }
        }
    }
    return lines;
}
async function compareTraces (fa, fb) {
    const ra = readline.createInterface({input: fs.createReadStream(fa), crlfDelay: Infinity})[Symbol.asyncIterator]();
    const rb = readline.createInterface({input: fs.createReadStream(fb), crlfDelay: Infinity})[Symbol.asyncIterator]();
    let n = 0;
    for (;;) {
        const [x, y] = await Promise.all([ra.next(), rb.next()]);
        if (x.done || y.done) return {n, diff: x.done === y.done ? null : `${x.done ? 'reference' : 'scrust'} trace ends after ${n} lines`};
        n++;
        if (x.value === y.value) continue;
        const a = normalise(JSON.parse(x.value));
        const b = normalise(JSON.parse(y.value));
        if (JSON.stringify(a) === JSON.stringify(b)) continue;
        return {n, diff: `frame ${a.frame}:\n      ` + describe(a, b).join('\n      ')};
    }
}

async function main () {
    fs.mkdirSync(GOT, {recursive: true});
    fs.mkdirSync(BUILD, {recursive: true});
    const files = projects();
    if (mode === 'scrust') {
        execFileSync('cargo', ['build', '-q', '--release', '-p', 'scrust-cli'], {cwd: ROOT, stdio: 'inherit'});
        let idx = 0;
        const worker = async () => { while (idx < files.length) console.log(await scrustOne(files[idx++])); };
        await Promise.all(Array.from({length: opt.jobs}, worker));
        return;
    }
    if (mode !== 'compare') { console.error('mode: scrust | compare'); process.exit(2); }
    const counts = {};
    for (const file of files) {
        const base = baseOf(file);
        let status; let msg = '';
        if (fs.existsSync(path.join(REF, `${base}.error`))) { status = 'REFERR'; msg = fs.readFileSync(path.join(REF, `${base}.error`), 'utf8').slice(0, 300); } else if (!fs.existsSync(path.join(REF, `${base}.done`))) status = 'NOREF';
        else if (fs.existsSync(path.join(GOT, `${base}.error`))) { status = 'SCRERR'; msg = fs.readFileSync(path.join(GOT, `${base}.error`), 'utf8').slice(-300); } else if (!fs.existsSync(path.join(GOT, `${base}.done`))) status = 'NOSCR';
        else {
            const c = await compareTraces(path.join(REF, `${base}.ref.jsonl`), path.join(GOT, `${base}.jsonl`));
            status = c.diff ? 'DIFF' : 'OK';
            msg = c.diff || `${c.n} trace lines`;
            if (opt.pixels) {
                const shots = fs.readdirSync(REF).filter(f => f.startsWith(`${base}-f`) && f.endsWith('.png')).sort();
                const pairs = shots.filter(f => fs.existsSync(path.join(GOT, f)));
                if (pairs.length) {
                    const r = spawnSync('python3', ['-P', path.join(__dirname, 'pixdiff.py'), REF, GOT, ...pairs], {encoding: 'utf8'});
                    msg += `\n      pixels: ${(r.stdout || r.stderr).trim()}`;
                }
            }
        }
        counts[status] = (counts[status] || 0) + 1;
        console.log(`${status.padEnd(6)} ${path.basename(file)}${msg ? '\n      ' + msg : ''}`);
    }
    console.log('\n' + Object.entries(counts).map(([k, v]) => `${k}: ${v}`).join('  ') + `  (of ${files.length})`);
}
main();
