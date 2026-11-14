// Real-project benchmark: TurboWarp's compiler vs scrust (native and wasm).
//
//   node conformance/bench/real.js <dir|file.sb3>... [--frames N] [--budget MS]
//        [--engines tw,native,wasm] [--only substr] [--list a,b] [--rebuild] [--no-build] [--timeout S] [--build-only] [--repeat N]
//
// Every engine runs headless (no renderer, no audio) and deterministically:
// virtual clock, seeded random numbers, one scheduler pass per frame, so all
// of them do the same work per frame. Only the scheduler step is timed
// (TurboWarp: `Sequencer.stepThreads`; scrust: `sched::step`). An engine stops
// after N frames or once it spent the budget; the comparison uses the frames
// every engine completed. TurboWarp compiles scripts lazily, so the first
// frames include its JIT compilation; the table also shows the time without
// the first `--warmup` frames (default 10).
//
// TurboWarp: SCRUST_REF_VM (default /Users/kobajin/repo/scratch-vm-develop).
// Results: conformance/out/bench-real/<project>.json and summary.json.
const fs = require('fs');
const path = require('path');
const {spawnSync, spawn} = require('child_process');

const ROOT = path.join(__dirname, '..', '..');
const OUT = path.join(ROOT, 'conformance', 'out', 'bench-real');
const BUILD = path.join(OUT, 'build');
const SCRUST = path.join(ROOT, 'target', 'release', 'scrust');
const VM = process.env.SCRUST_REF_VM || '/Users/kobajin/repo/scratch-vm-develop';
const EPOCH = 1577836800000;

function mulberry32 (a) {
    return function () {
        a |= 0; a = (a + 0x6D2B79F5) | 0;
        let t = Math.imul(a ^ (a >>> 15), 1 | a);
        t = (t + Math.imul(t ^ (t >>> 7), 61 | t)) ^ t;
        return ((t ^ (t >>> 14)) >>> 0) / 4294967296;
    };
}

// ------------------------------------------------------------ child: TurboWarp
async function childTw (file, frames, budget) {
    const log = require(path.join(VM, 'src/util/log'));
    for (const k of ['warn', 'error', 'info', 'log', 'debug']) if (log[k]) log[k] = () => {};
    const uidPath = require.resolve(path.join(VM, 'src/util/uid'));
    let uidCounter = 0;
    require.cache[uidPath] = {id: uidPath, filename: uidPath, loaded: true, exports: () => `uid-${uidCounter++}`, children: [], paths: []};
    const VirtualMachine = require(path.join(VM, 'src/virtual-machine'));
    const vm = new VirtualMachine();
    vm.setCompilerOptions({enabled: true, warpTimer: false});
    const rt = vm.runtime;
    await vm.loadProject(fs.readFileSync(file));
    const realNow = performance.now.bind(performance);
    let now = EPOCH;
    Date.now = () => now;
    performance.now = () => now;
    const prng = mulberry32(1);
    Math.random = () => prng();
    let timers = [];
    let seq = 0;
    global.setTimeout = (fn, ms) => {
        const d = Number(ms);
        const t = {at: now + (d > 0 ? d : 0), seq: seq++, fn};
        timers.push(t);
        return t;
    };
    global.clearTimeout = t => { timers = timers.filter(x => x !== t); };
    rt.sequencer.timer = {start () { this.n = 0; }, timeElapsed () { return this.n++ >= 1 ? Infinity : 0; }};
    const framerate = rt.frameLoop.framerate || 60;
    const interval = 1000 / framerate;
    rt.currentStepTime = interval;
    rt.updateCurrentMSecs();
    rt.ioDevices.clock.resetProjectTimer();
    const step = [];
    const seqr = rt.sequencer;
    const inner = seqr.stepThreads.bind(seqr);
    seqr.stepThreads = function () {
        const t0 = realNow();
        const r = inner();
        step.push(realNow() - t0);
        return r;
    };
    vm.greenFlag();
    let total = 0;
    for (let f = 0; f < frames; f++) {
        now += interval;
        await new Promise(r => setImmediate(r));
        const due = timers.filter(t => t.at <= now).sort((a, b) => (a.at - b.at) || (a.seq - b.seq));
        timers = timers.filter(t => t.at > now);
        for (const t of due) t.fn();
        await new Promise(r => setImmediate(r));
        const n = step.length;
        try { rt._step(); } catch (e) { /* as in the browser: the frame is lost */ }
        for (let i = n; i < step.length; i++) total += step[i];
        if (total > budget) break;
    }
    return {step};
}

// ------------------------------------------------------------ child: scrust wasm
async function childWasm (file, frames, budget) {
    const bytes = fs.readFileSync(file);
    const mod = await WebAssembly.compile(bytes);
    const imports = {};
    for (const imp of WebAssembly.Module.imports(mod)) {
        imports[imp.module] = imports[imp.module] || {};
        if (imp.kind === 'function') imports[imp.module][imp.name] = () => { throw new Error(`import ${imp.module}.${imp.name} called`); };
    }
    const inst = await WebAssembly.instantiate(mod, imports);
    const ex = inst.exports;
    ex.scrust_bench_init(1);
    const step = [];
    let total = 0;
    for (let f = 0; f < frames; f++) {
        const t0 = performance.now();
        ex.scrust_bench_step();
        const ms = performance.now() - t0;
        step.push(ms);
        total += ms;
        if (total > budget) break;
    }
    return {step};
}

if (process.argv[2] === '--child') {
    const [, , , engine, file, frames, budget] = process.argv;
    const f = engine === 'tw' ? childTw : childWasm;
    f(file, Number(frames), Number(budget)).then(r => {
        process.stdout.write(JSON.stringify(r) + '\n', () => process.exit(0));
    }).catch(e => { console.error(e && e.stack || e); process.exit(1); });
    return;
}

// ------------------------------------------------------------ driver
const argv = process.argv.slice(2);
const opt = {frames: 150, budget: 30000, warmup: 10, engines: ['tw', 'native', 'wasm'], only: null, list: null, rebuild: false, build: true, inputs: [], timeout: 180, repeat: 1};
for (let i = 0; i < argv.length; i++) {
    const a = argv[i];
    if (a === '--frames') opt.frames = Number(argv[++i]);
    else if (a === '--budget') opt.budget = Number(argv[++i]);
    else if (a === '--warmup') opt.warmup = Number(argv[++i]);
    else if (a === '--engines') opt.engines = argv[++i].split(',');
    else if (a === '--only') opt.only = argv[++i];
    else if (a === '--list') opt.list = argv[++i].split(',');
    else if (a === '--rebuild') opt.rebuild = true;
    else if (a === '--no-build') opt.build = false;
    else if (a === '--timeout') opt.timeout = Number(argv[++i]);
    else if (a === '--build-only') opt.buildOnly = true;
    else if (a === '--repeat') opt.repeat = Number(argv[++i]);
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
    if (opt.list) files = files.filter(f => opt.list.includes(baseOf(f)));
    return files;
}

// The fastest of `--repeat` runs (other processes only ever add time).
async function best (cmd, args) {
    let r = null;
    for (let k = 0; k < opt.repeat; k++) {
        const x = await runJson(cmd, args);
        if (!r || (x.step && (!r.step || sum(x.step) < sum(r.step)))) r = x;
    }
    return r;
}
const runJson = (cmd, args) => new Promise(resolve => {
    const p = spawn(cmd, args, {stdio: ['ignore', 'pipe', 'pipe']});
    let out = ''; let err = '';
    p.stdout.on('data', d => { out += d; });
    p.stderr.on('data', d => { err = (err + d).slice(-2000); });
    const timer = setTimeout(() => p.kill('SIGKILL'), opt.timeout * 1000);
    p.on('close', code => {
        clearTimeout(timer);
        const line = out.trim().split('\n').pop();
        try { resolve(JSON.parse(line)); } catch (e) { resolve({error: `exit ${code}: ${err.slice(-400)}`}); }
    });
});

function build (file, target) {
    const base = baseOf(file);
    const dir = path.join(BUILD, `${base}-${target}`);
    const marker = path.join(dir, '.built');
    if (!opt.build || (!opt.rebuild && fs.existsSync(marker) && fs.statSync(marker).mtimeMs > fs.statSync(SCRUST).mtimeMs)) {
        return locate(dir, target);
    }
    fs.rmSync(dir, {recursive: true, force: true});
    const args = ['build', file, '-o', dir, '--release', '--headless', '--allow-unsupported'];
    if (target === 'wasm') args.push('--target', 'wasm');
    const r = spawnSync(SCRUST, args, {encoding: 'utf8'});
    if (r.status !== 0) return {error: `build: ${(r.stderr || '').slice(-600)}`};
    fs.writeFileSync(marker, '');
    return locate(dir, target);
}
function locate (dir, target) {
    if (!fs.existsSync(dir)) return {error: 'not built'};
    const files = fs.readdirSync(dir).map(f => path.join(dir, f)).filter(f => fs.statSync(f).isFile());
    const exe = target === 'wasm' ? files.find(f => f.endsWith('.wasm')) : files.find(f => (fs.statSync(f).mode & 0o111) && !f.endsWith('.html'));
    return exe ? {exe} : {error: 'no output'};
}

const sum = a => a.reduce((x, y) => x + y, 0);
async function main () {
    fs.mkdirSync(BUILD, {recursive: true});
    if (opt.build && opt.engines.some(e => e !== 'tw')) spawnSync('cargo', ['build', '-q', '--release', '-p', 'scrust-cli'], {cwd: ROOT, stdio: 'inherit'});
    const rows = [];
    const pad = (s, n) => String(s).padStart(n);
    console.log(`${'project'.padEnd(44)}${pad('frames', 7)}${pad('TW ms', 10)}${pad('native', 10)}${pad('wasm', 10)}${pad('nat/TW', 8)}${pad('wasm/TW', 8)}${pad('nat/TW*', 8)}${pad('wasm/TW*', 9)}`);
    for (const file of projects()) {
        const base = baseOf(file);
        const res = {};
        for (const e of opt.engines) {
            if (e === 'tw') {
                const prev = path.join(OUT, `${base}.json`);
                // The reference does not change: reuse earlier TurboWarp runs of at least as many frames.
                let old = null;
                try { old = JSON.parse(fs.readFileSync(prev, 'utf8')).tw; } catch (_) { /* none */ }
                if (old && old.step && (old.step.length >= opt.frames || sum(old.step) > opt.budget) && !process.env.BENCH_TW_AGAIN) res.tw = old;
                else res.tw = await best(process.execPath, [__filename, '--child', 'tw', file, String(opt.frames), String(opt.budget)]);
            } else if (opt.buildOnly) {
                res[e] = build(file, e);
                continue;
            } else if (e === 'native') {
                const b = build(file, 'native');
                res.native = b.error ? b : await best(b.exe, ['--deterministic', '--frames', String(opt.frames), '--bench-json', '--bench-budget', String(opt.budget)]);
            } else if (e === 'wasm') {
                const b = build(file, 'wasm');
                res.wasm = b.error ? b : await best(process.execPath, [__filename, '--child', 'wasm', b.exe, String(opt.frames), String(opt.budget)]);
            }
        }
        if (opt.buildOnly) {
            console.log(`${base}: ${Object.entries(res).map(([k, v]) => `${k} ${v.error ? v.error.slice(0, 200) : 'built'}`).join(', ')}`);
            continue;
        }
        let prevAll = {};
        try { prevAll = JSON.parse(fs.readFileSync(path.join(OUT, `${base}.json`), 'utf8')); } catch (_) { /* none */ }
        fs.writeFileSync(path.join(OUT, `${base}.json`), JSON.stringify(Object.assign(prevAll, res)));
        const ok = Object.values(res).filter(r => r && r.step);
        const n = ok.length ? Math.min(...ok.map(r => r.step.length)) : 0;
        const tot = k => (res[k] && res[k].step ? sum(res[k].step.slice(0, n)) : NaN);
        const warm = k => (res[k] && res[k].step ? sum(res[k].step.slice(Math.min(opt.warmup, n - 1), n)) : NaN);
        const row = {project: base, frames: n, tw: tot('tw'), native: tot('native'), wasm: tot('wasm'), twW: warm('tw'), nativeW: warm('native'), wasmW: warm('wasm')};
        for (const k of opt.engines) if (res[k] && res[k].error) row[`${k}Error`] = res[k].error;
        rows.push(row);
        const f = x => (Number.isFinite(x) ? x.toFixed(1) : '-');
        const r = (a, b) => (Number.isFinite(a) && Number.isFinite(b) && b > 0 ? (a / b).toFixed(2) + 'x' : '-');
        console.log(`${base.slice(0, 43).padEnd(44)}${pad(n, 7)}${pad(f(row.tw), 10)}${pad(f(row.native), 10)}${pad(f(row.wasm), 10)}${pad(r(row.tw, row.native), 8)}${pad(r(row.tw, row.wasm), 8)}${pad(r(row.twW, row.nativeW), 8)}${pad(r(row.twW, row.wasmW), 9)}`);
        for (const k of opt.engines) if (row[`${k}Error`]) console.log(`    ${k}: ${row[`${k}Error`].replace(/\s+/g, ' ').slice(0, 300)}`);
    }
    // Geometric means over projects where TurboWarp spent a measurable time.
    const geo = (key, ref) => {
        const xs = rows.filter(r => Number.isFinite(r[key]) && Number.isFinite(r[ref]) && r[ref] > 20 && r[key] > 0).map(r => Math.log(r[ref] / r[key]));
        return xs.length ? `${Math.exp(sum(xs) / xs.length).toFixed(2)}x over ${xs.length}` : '-';
    };
    console.log(`\ngeomean speed-up vs TurboWarp (projects with TW > 20 ms): native ${geo('native', 'tw')}, wasm ${geo('wasm', 'tw')}; without warm-up: native ${geo('nativeW', 'twW')}, wasm ${geo('wasmW', 'twW')}`);
    fs.writeFileSync(path.join(OUT, 'summary.json'), JSON.stringify(rows, null, 1));
}
main();
