// Deterministic headless tracer for scratch-vm (the ground truth).
//   node trace.js <project.json|.sb3> --frames N [--seed S] [--fps F] [--max-passes P] [--turbo] [--answers 'a|b']
// Output: one JSON line per frame, identical in format to `scrust trace`.
const fs = require('fs');
const path = require('path');

// SCRUST_REF_VM points at another checkout (e.g. TurboWarp/scratch-vm, whose
// compiler is enabled by default).
const VENDOR = process.env.SCRUST_REF_VM || path.join(__dirname, '..', 'vendor', 'scratch-vm');
const EPOCH = 1577836800000; // keep in sync with scrust-runtime DETERMINISTIC_EPOCH_MS

function mulberry32 (a) {
    return function () {
        a |= 0; a = (a + 0x6D2B79F5) | 0;
        let t = Math.imul(a ^ (a >>> 15), 1 | a);
        t = (t + Math.imul(t ^ (t >>> 7), 61 | t)) ^ t;
        return ((t ^ (t >>> 14)) >>> 0) / 4294967296;
    };
}

const encNum = x => (Object.is(x, -0) ? '-0' : String(x));
const enc = v => {
    if (typeof v === 'number') return {t: 'num', v: encNum(v)};
    if (typeof v === 'boolean') return {t: 'bool', v};
    if (typeof v === 'string') return {t: 'str', v};
    return {t: typeof v, v: String(v)};
};

async function trace (input, opts) {
    const log = require(path.join(VENDOR, 'src/util/log'));
    for (const k of ['warn', 'error', 'info', 'log', 'debug']) if (log[k]) log[k] = () => {};
    // scratch-vm draws its internal ids (targets, clones, say bubbles) from
    // Math.random. That is unobservable to projects but would shift the seeded
    // sequence, so ids come from a counter instead.
    const uidPath = require.resolve(path.join(VENDOR, 'src/util/uid'));
    let uidCounter = 0;
    require.cache[uidPath] = {id: uidPath, filename: uidPath, loaded: true, exports: () => `uid-${uidCounter++}`, children: [], paths: []};
    const VirtualMachine = require(path.join(VENDOR, 'src/virtual-machine'));
    const vm = new VirtualMachine();
    const rt = vm.runtime;
    await vm.loadProject(input);

    // ---- determinism --------------------------------------------------------
    let now = EPOCH;
    const realSetImmediate = setImmediate;
    Date.now = () => now;
    let rngCalls = 0;
    const prng = mulberry32(opts.seed);
    Math.random = () => { rngCalls++; return prng(); };
    let timers = [];
    let seq = 0;
    global.setTimeout = (fn, ms) => {
        const d = Number(ms);
        const t = {at: now + (d > 0 ? d : 0), seq: seq++, fn};
        timers.push(t);
        return t;
    };
    global.clearTimeout = t => { timers = timers.filter(x => x !== t); };
    const flush = () => new Promise(r => realSetImmediate(r));

    // Scheduler: count passes instead of wall time; count passes via the profiler hooks.
    let passes = 0;
    const maxPasses = opts.maxPasses;
    rt.sequencer.timer = {start () { this.n = 0; }, timeElapsed () { return this.n++ >= maxPasses ? Infinity : 0; }};
    const ids = {};
    rt.profiler = {
        idByName (n) { if (!(n in ids)) ids[n] = Object.keys(ids).length; return ids[n]; },
        start (id) { if (id === ids['Sequencer.stepThreads#inner']) passes++; },
        stop () {}, increment () {}, frame () { return {count: 0}; }, reportFrames () {}
    };

    const interval = 1000 / opts.fps;
    rt.currentStepTime = interval;
    rt.turboMode = !!opts.turbo;
    const answers = opts.answers.slice();
    let pendingQuestion = false;
    rt.on('QUESTION', q => { pendingQuestion = q !== null; });

    rt.updateCurrentMSecs();
    rt.ioDevices.clock.resetProjectTimer();
    vm.greenFlag();
    const out = [];
    for (let frame = 1; frame <= opts.frames; frame++) {
        now += interval;
        await flush();
        if (pendingQuestion && answers.length > 0) {
            pendingQuestion = false;
            rt.emit('ANSWER', answers.shift());
            await flush();
        }
        const due = timers.filter(t => t.at <= now).sort((a, b) => (a.at - b.at) || (a.seq - b.seq));
        timers = timers.filter(t => t.at > now);
        for (const t of due) t.fn();
        await flush();
        passes = 0;
        // An exception escaping _step is swallowed by the browser's timer loop.
        try { rt._step(); } catch (e) { if (process.env.SCRUST_TRACE_ERRORS) console.error(e); }
        const live = rt.threads.filter(t => !t.isKilled && t.status !== 4 && t.stack.length > 0).length;
        const targets = rt.executableTargets.map(t => {
            const st = t.getCustomState('Scratch.looks');
            const vars = [];
            const lists = [];
            for (const id of Object.keys(t.variables)) {
                const v = t.variables[id];
                if (v.type === '') vars.push([v.name, enc(v.value)]);
                else if (v.type === 'list') lists.push([v.name, v.value.map(enc)]);
            }
            return {
                name: t.sprite.name, clone: !t.isOriginal, x: encNum(t.x), y: encNum(t.y), dir: encNum(t.direction), size: encNum(t.size),
                costume: t.currentCostume, visible: t.visible,
                say: st && st.text !== '' ? {type: st.type, text: st.text} : null, vars, lists
            };
        });
        out.push(JSON.stringify({frame, threads: live, passes, rng: rngCalls, targets}));
    }
    return out;
}

module.exports = {trace};

if (require.main === module) {
    const argv = process.argv.slice(2);
    const opts = {frames: 10, seed: 1, fps: 30, maxPasses: 1, turbo: false, answers: []};
    let file = null;
    for (let i = 0; i < argv.length; i++) {
        const a = argv[i];
        if (a === '--frames') opts.frames = Number(argv[++i]);
        else if (a === '--seed') opts.seed = Number(argv[++i]);
        else if (a === '--fps') opts.fps = Number(argv[++i]);
        else if (a === '--max-passes') opts.maxPasses = Number(argv[++i]);
        else if (a === '--turbo') opts.turbo = true;
        else if (a === '--answers') opts.answers = argv[++i].split('|');
        else file = a;
    }
    const buf = fs.readFileSync(file);
    const input = buf[0] === 0x50 && buf[1] === 0x4b ? buf : buf.toString('utf8');
    const realStdoutWrite = process.stdout.write.bind(process.stdout);
    trace(input, opts).then(lines => { realStdoutWrite(lines.join('\n') + '\n', () => process.exit(0)); })
        .catch(e => { console.error(e); process.exit(1); });
}
