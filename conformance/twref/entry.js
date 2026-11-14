// TurboWarp reference run in a browser, with scratch-render attached, driven
// deterministically like harness/trace.js (same trace line format).
// Query parameters:
//   list=a.sb3|b.sb3  projects (served from /p/), run one after another
//   frames=N          frames per project (default 30)
//   shots=1,5,9       frames to save as PNG, or every=N for every Nth frame
//   list=*            every project in the served directory (skipping ones
//                     that already have results unless redo=1)
//   trace=0           do not save the trace; traceEvery=N saves every Nth frame
//   seed, maxPasses as in trace.js; fps overrides the project's framerate; suffix=S appends S to output names
// Results go to conformance/out/twref/results/<project>.ref.jsonl,
// <project>-f000123.png and <project>.done (or .error).
const VM = require('scratch-vm');
const Renderer = require('scratch-render');
const Storage = require('@turbowarp/scratch-storage');
const {BitmapAdapter} = require('@turbowarp/scratch-svg-renderer');

const q = new URLSearchParams(location.search);
let list = (q.get('list') || '').split('|').filter(Boolean);
const index = Number(q.get('i') || 0);
const opts = {
    frames: Number(q.get('frames') || 30),
    seed: Number(q.get('seed') || 1),
    // Default: the project's own framerate (TurboWarp config comment).
    fps: q.get('fps') ? Number(q.get('fps')) : null,
    maxPasses: Number(q.get('maxPasses') || 1),
    trace: q.get('trace') !== '0',
    traceEvery: Number(q.get('traceEvery') || 1),
    // Lists longer than this are traced as [{t:'hash', v:'<length>:<fnv1a>'}].
    hashOver: Number(q.get('hashOver') || 0),
    every: Number(q.get('every') || 0),
    // timing=1: no trace or screenshots; the real time of every
    // Sequencer.stepThreads call is saved to <project><suffix>.time.json.
    timing: q.get('timing') === '1',
    shots: new Set((q.get('shots') || '').split(',').filter(Boolean).map(Number))
};
const EPOCH = 1577836800000; // harness/trace.js
const log = s => { document.getElementById('log').textContent += s + '\n'; };

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
// FNV-1a over the JSON of each traced item (same as run.js).
const hashList = items => {
    let h = 0x811c9dc5;
    for (const it of items) {
        let e = enc(it);
        // run.js's canonical-number normalisation, before hashing.
        if (e.t === 'str' && e.v !== '' && e.v !== 'NaN' && String(Number(e.v)) === e.v) e = {t: 'num', v: e.v};
        const str = JSON.stringify(e);
        for (let i = 0; i < str.length; i++) {
            h ^= str.charCodeAt(i);
            h = Math.imul(h, 0x01000193) >>> 0;
        }
    }
    return [{t: 'hash', v: `${items.length}:${h}`}];
};
const save = (name, body, append) => fetch(`/save?name=${encodeURIComponent(name)}${append ? '&append=1' : ''}`, {method: 'POST', body});
const tick = (() => {
    const ch = new MessageChannel();
    const queue = [];
    ch.port1.onmessage = () => queue.shift()();
    return () => new Promise(r => { queue.push(r); ch.port2.postMessage(0); });
})();
const realSetTimeout = window.setTimeout.bind(window);
const realNow = performance.now.bind(performance);
const sleep = ms => new Promise(r => realSetTimeout(r, ms));

async function run (name) {
    const base = name.replace(/\.sb3$/, '').replace(/[^\w.-]+/g, '_') + (q.get('suffix') || '');
    const canvas = document.getElementById('c');
    const vm = new VM();
    const renderer = new Renderer(canvas);
    const dpr = window.devicePixelRatio || 1;
    renderer.resize(480 / dpr, 360 / dpr);
    vm.attachRenderer(renderer);
    window.vm = vm; // for inspection from the console
    vm.attachStorage(new (Storage.default || Storage)());
    vm.attachV2BitmapAdapter(new BitmapAdapter());
    const rt = vm.runtime;
    const buf = await (await fetch(`/p/${encodeURIComponent(name)}`)).arrayBuffer();
    await vm.loadProject(buf);
    // SVG skins finish loading asynchronously (<img> onload).
    for (let i = 0; i < 400; i++) {
        const pending = renderer._allSkins.filter(s => s && '_svgImageLoaded' in s && !s._svgImageLoaded && s._size[0] > 0 && s._size[1] > 0);
        if (pending.length === 0) break;
        await sleep(25);
    }

    // ---- determinism (harness/trace.js) -------------------------------------
    let now = EPOCH;
    Date.now = () => now;
    performance.now = () => now;
    let rngCalls = 0;
    const prng = mulberry32(opts.seed);
    const rngSites = q.get('debugRng') ? {} : null;
    Math.random = () => {
        rngCalls++;
        if (rngSites) {
            // Compiled scripts are named after their blocks; keep the frames above Math.random.
            const st = (new Error().stack || '').split('\n').slice(2, 5).map(l => l.trim().replace(/\(.*\)/, '').replace(/^at /, '')).join(' < ');
            rngSites[st] = (rngSites[st] || 0) + 1;
        }
        return prng();
    };
    let timers = [];
    let seq = 0;
    window.setTimeout = (fn, ms) => {
        const d = Number(ms);
        const t = {at: now + (d > 0 ? d : 0), seq: seq++, fn};
        timers.push(t);
        return t;
    };
    window.clearTimeout = t => { timers = timers.filter(x => x !== t); };
    let passes = 0;
    const maxPasses = opts.maxPasses;
    rt.sequencer.timer = {start () { this.n = 0; }, timeElapsed () { return this.n++ >= maxPasses ? Infinity : 0; }};
    const ids = {};
    rt.profiler = {
        idByName (n) { if (!(n in ids)) ids[n] = Object.keys(ids).length; return ids[n]; },
        start (id) { if (id === ids['Sequencer.stepThreads#inner']) passes++; },
        stop () {}, increment () {}, frame () { return {count: 0}; }, reportFrames () {}
    };
    const framerate = opts.fps || rt.frameLoop.framerate || 60; // 0 = display rate
    const interval = 1000 / framerate;
    rt.currentStepTime = interval;
    rt.updateCurrentMSecs();
    rt.ioDevices.clock.resetProjectTimer();
    const stepMs = [];
    if (opts.timing) {
        opts.trace = false;
        opts.every = 0;
        opts.shots.clear();
        const seqr = rt.sequencer;
        const inner = seqr.stepThreads.bind(seqr);
        seqr.stepThreads = function () {
            const t0 = realNow();
            const r = inner();
            stepMs.push(realNow() - t0);
            return r;
        };
    }
    vm.greenFlag();

    const gl = renderer.gl;
    const px = new Uint8Array(480 * 360 * 4);
    const flip = document.createElement('canvas');
    flip.width = 480; flip.height = 360;
    const fctx = flip.getContext('2d');
    let batch = [];
    let batchChars = 0;
    if (opts.trace) await save(`${base}.ref.jsonl`, '');
    for (let frame = 1; frame <= opts.frames; frame++) {
        now += interval;
        await tick();
        const due = timers.filter(t => t.at <= now).sort((a, b) => (a.at - b.at) || (a.seq - b.seq));
        timers = timers.filter(t => t.at > now);
        for (const t of due) t.fn();
        await tick();
        passes = 0;
        try { rt._step(); } catch (e) { console.error(e); }
        if (opts.trace && frame % opts.traceEvery === 0) {
            const live = rt.threads.filter(t => !t.isKilled && t.status !== 4 && t.stack.length > 0).length;
            const targets = rt.executableTargets.map(t => {
                const st = t.getCustomState('Scratch.looks');
                const vars = [];
                const lists = [];
                for (const id of Object.keys(t.variables)) {
                    const v = t.variables[id];
                    if (v.type === '') vars.push([v.name, enc(v.value)]);
                    else if (v.type === 'list') lists.push([v.name, opts.hashOver && v.value.length > opts.hashOver ? hashList(v.value) : v.value.map(enc)]);
                }
                return {
                    name: t.sprite.name, clone: !t.isOriginal, x: encNum(t.x), y: encNum(t.y), dir: encNum(t.direction), size: encNum(t.size),
                    costume: t.currentCostume, visible: t.visible,
                    say: st && st.text !== '' ? {type: st.type, text: st.text} : null, vars, lists
                };
            });
            const line = JSON.stringify({frame, threads: live, passes, rng: rngCalls, targets});
            batch.push(line);
            batchChars += line.length;
            if (batch.length >= 10 || batchChars > 32e6) {
                batchChars = 0;
                await save(`${base}.ref.jsonl`, batch.join('\n') + '\n', true);
                batch = [];
            }
        }
        if (opts.shots.has(frame) || (opts.every > 0 && frame % opts.every === 0)) {
            // draw() skips unchanged frames, and the canvas is not preserved
            // between tasks: force it.
            renderer.dirty = true;
            renderer.draw();
            gl.readPixels(0, 0, 480, 360, gl.RGBA, gl.UNSIGNED_BYTE, px);
            const id = fctx.createImageData(480, 360);
            for (let y = 0; y < 360; y++) id.data.set(px.subarray((359 - y) * 1920, (360 - y) * 1920), y * 1920);
            for (let i = 3; i < id.data.length; i += 4) id.data[i] = 255;
            fctx.putImageData(id, 0, 0);
            const blob = await new Promise(r => flip.toBlob(r, 'image/png'));
            await save(`${base}-f${String(frame).padStart(6, '0')}.png`, blob);
        }
    }
    if (batch.length) await save(`${base}.ref.jsonl`, batch.join('\n') + '\n', true);
    if (opts.timing) await save(`${base}.time.json`, JSON.stringify({engine: 'turbowarp', frames: opts.frames, step: stepMs}));
    if (rngSites) await save(`${base}.rng.json`, JSON.stringify(rngSites, null, 1));
    await save(`${base}.done`, `${opts.frames} frames`);
}

(async () => {
    if (list.length === 1 && list[0] === '*') list = await (await fetch('/list')).json();
    if (index < list.length && q.get('redo') !== '1') {
        const base = list[index].replace(/\.sb3$/, '').replace(/[^\w.-]+/g, '_') + (q.get('suffix') || '');
        const done = await fetch(`/r/${encodeURIComponent(base)}.done`);
        const failed = await fetch(`/r/${encodeURIComponent(base)}.error`);
        if (done.ok || failed.ok) { q.set('i', String(index + 1)); location.search = q.toString(); return; }
    }
    if (index >= list.length) { log('all done'); document.title = 'done'; return; }
    const name = list[index];
    log(`running ${name}`);
    try {
        await run(name);
    } catch (e) {
        const base = name.replace(/\.sb3$/, '').replace(/[^\w.-]+/g, '_') + (q.get('suffix') || '');
        await save(`${base}.error`, String(e && e.stack || e));
    }
    if (q.get('hold')) { log('held'); return; }
    q.set('i', String(index + 1));
    location.search = q.toString();
})();
