// Checks that optimised builds behave exactly like a reference build on real
// projects: deterministic headless traces (one line per frame, as `--trace`
// prints them), hashed per frame, of this checkout's native and wasm builds
// against the same builds of another checkout. (Native and wasm builds may
// differ from each other in the last bits of trigonometric results: wasm
// uses the libm crate, see docs/decisions/math-libm.md.)
//
//   node conformance/bench/verify.js <dir|file.sb3>... --ref <other checkout>
//        [--frames N] [--list a,b] [--only s] [--engines native,wasm]
//
// Builds come from conformance/bench/real.js (conformance/out/bench-real/build
// of this checkout and of the reference checkout); run it first in both.
const fs = require('fs');
const path = require('path');
const crypto = require('crypto');
const {spawn} = require('child_process');

const ROOT = path.join(__dirname, '..', '..');
const argv = process.argv.slice(2);
const opt = {frames: 150, ref: null, list: null, only: null, engines: ['native', 'wasm'], inputs: []};
for (let i = 0; i < argv.length; i++) {
    const a = argv[i];
    if (a === '--frames') opt.frames = Number(argv[++i]);
    else if (a === '--ref') opt.ref = argv[++i];
    else if (a === '--list') opt.list = argv[++i].split(',');
    else if (a === '--only') opt.only = argv[++i];
    else if (a === '--engines') opt.engines = argv[++i].split(',');
    else opt.inputs.push(a);
}
const baseOf = f => path.basename(f).replace(/\.sb3$/, '').replace(/[^\w.-]+/g, '_');

if (argv[0] === '--wasm-child') {
    // node verify.js --wasm-child file.wasm frames: one trace line per frame on stdout.
    (async () => {
        const mod = await WebAssembly.compile(fs.readFileSync(argv[1]));
        const imports = {};
        for (const imp of WebAssembly.Module.imports(mod)) {
            imports[imp.module] = imports[imp.module] || {};
            if (imp.kind === 'function') imports[imp.module][imp.name] = () => { throw new Error(`import ${imp.name}`); };
        }
        const ex = (await WebAssembly.instantiate(mod, imports)).exports;
        ex.scrust_bench_init(1);
        // Lines go out as raw bytes, one write each (they can be huge).
        const fd = 1;
        for (let f = 0; f < Number(argv[2]); f++) {
            ex.scrust_bench_step();
            const n = ex.scrust_bench_trace();
            fs.writeSync(fd, new Uint8Array(ex.memory.buffer, ex.scrust_bench_trace_ptr(), n));
            fs.writeSync(fd, '\n');
        }
        process.exit(0);
    })().catch(e => { console.error(e && e.stack || e); process.exit(1); });
    return;
}

// Per-frame hashes of a trace, streamed (traces of big lists are huge).
const hashes = (cmd, args) => new Promise(resolve => {
    const p = spawn(cmd, args, {stdio: ['ignore', 'pipe', 'pipe']});
    const out = [];
    let buf = '';
    let err = '';
    p.stdout.on('data', d => {
        buf += d;
        let i;
        while ((i = buf.indexOf('\n')) >= 0) {
            out.push(crypto.createHash('sha1').update(buf.slice(0, i)).digest('hex').slice(0, 16));
            buf = buf.slice(i + 1);
        }
    });
    p.stderr.on('data', d => { err = (err + d).slice(-1000); });
    const timer = setTimeout(() => p.kill('SIGKILL'), 900 * 1000);
    p.on('close', code => { clearTimeout(timer); resolve(code === 0 ? {h: out} : {error: `exit ${code} ${err}`}); });
});
const exeIn = (root, base, target) => {
    const dir = path.join(root, 'conformance', 'out', 'bench-real', 'build', `${base}-${target}`);
    if (!fs.existsSync(dir)) return null;
    const files = fs.readdirSync(dir).map(f => path.join(dir, f)).filter(f => fs.statSync(f).isFile());
    return target === 'wasm' ? files.find(f => f.endsWith('.wasm')) : files.find(f => (fs.statSync(f).mode & 0o111) && !f.endsWith('.html'));
};
const traceOf = (exe, target) => (target === 'wasm' ?
    hashes(process.execPath, [__filename, '--wasm-child', exe, String(opt.frames)]) :
    hashes(exe, ['--deterministic', '--trace', String(opt.frames)]));

async function main () {
    let files = [];
    for (const i of opt.inputs) {
        if (fs.statSync(i).isDirectory()) files.push(...fs.readdirSync(i).filter(f => f.endsWith('.sb3')).sort().map(f => path.join(i, f)));
        else files.push(i);
    }
    if (opt.only) files = files.filter(f => path.basename(f).includes(opt.only));
    if (opt.list) files = files.filter(f => opt.list.includes(baseOf(f)));
    let bad = 0;
    for (const file of files) {
        const base = baseOf(file);
        const parts = [];
        for (const t of opt.engines) {
            const refExe = exeIn(opt.ref, base, t);
            if (!refExe) { parts.push(`${t}: no reference build`); continue; }
            const ref = await traceOf(refExe, t);
            if (ref.error) { parts.push(`${t}: reference failed ${ref.error.slice(0, 200)}`); continue; }
            const exe = exeIn(ROOT, base, t);
            if (!exe) { parts.push(`${t}: not built`); continue; }
            const got = await traceOf(exe, t);
            if (got.error) { parts.push(`${t}: ${got.error.slice(0, 200)}`); bad++; continue; }
            const n = Math.max(ref.h.length, got.h.length);
            let first = -1;
            for (let i = 0; i < n; i++) if (ref.h[i] !== got.h[i]) { first = i; break; }
            if (first >= 0) { bad++; parts.push(`${t}: DIFF from frame ${first + 1}`); } else parts.push(`${t}: ok`);
        }
        console.log(`${base.padEnd(50)} ${parts.join('  ')}`);
    }
    console.log(bad ? `\n${bad} differences` : '\nall identical');
    process.exit(bad ? 1 : 0);
}
main();
