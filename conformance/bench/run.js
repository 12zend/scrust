// Benchmark: scratch-vm (interpreter) vs TurboWarp (JS JIT) vs scrust.
//   node run.js            runs all engines on all projects in ../out/bench_*.json
//   node run.js --engine vanilla|tw <file>   (internal: one measurement)
const fs = require('fs');
const path = require('path');
const {execFileSync} = require('child_process');
const ROOT = path.join(__dirname, '..', '..');

async function measure (engine, file) {
    const vendor = path.join(__dirname, '..', 'vendor', engine === 'tw' ? 'tw-scratch-vm' : 'scratch-vm');
    const log = require(path.join(vendor, 'src/util/log'));
    for (const k of ['warn', 'error', 'info', 'log', 'debug']) if (log[k]) log[k] = () => {};
    const VM = require(path.join(vendor, 'src/virtual-machine'));
    const vm = new VM();
    if (engine === 'tw') vm.setCompilerOptions({enabled: true, warpTimer: false});
    await vm.loadProject(fs.readFileSync(file, 'utf8'));
    const rt = vm.runtime;
    rt.currentStepTime = 1000 / 30;
    const stage = rt.getTargetForStage();
    const done = () => Object.values(stage.variables).some(v => v.name === 'done' && Number(v.value) === 1);
    const t0 = process.hrtime.bigint();
    vm.greenFlag();
    let steps = 0;
    while (!done() && steps < 100000) { rt._step(); steps++; }
    const ms = Number(process.hrtime.bigint() - t0) / 1e6;
    process.stdout.write(`${ms.toFixed(1)}\n`, () => process.exit(0));
}

if (process.argv[2] === '--engine') {
    measure(process.argv[3], process.argv[4]).catch(e => { console.error(e); process.exit(1); });
} else {
    const out = path.join(ROOT, 'conformance', 'out');
    execFileSync('node', [path.join(__dirname, 'projects.js')]);
    const files = fs.readdirSync(out).filter(f => /^bench_.*\.json$/.test(f)).sort();
    const rows = [];
    for (const f of files) {
        const file = path.join(out, f);
        const row = {project: f.replace(/^bench_|\.json$/g, '')};
        for (const engine of ['vanilla', 'tw']) {
            try { row[engine] = Number(execFileSync('node', [__filename, '--engine', engine, file], {timeout: 600000}).toString().trim()); } catch (e) { row[engine] = NaN; }
        }
        for (const [key, extra] of [['scrust', []], ['scrust_nounbox', ['--no-unbox']]]) {
            const res = execFileSync(path.join(ROOT, 'target/debug/scrust'), ['run', file, '--headless', ...extra, '--deterministic', '--frames', '3', '--', '--bench'], {stdio: ['ignore', 'pipe', 'pipe'], encoding: 'utf8'});
            row[key] = NaN;
        }
        rows.push(row);
    }
    // scrust prints timing on stderr; re-run capturing it.
    const {spawnSync} = require('child_process');
    for (const row of rows) {
        const file = path.join(out, `bench_${row.project}.json`);
        for (const [key, extra] of [['scrust', []], ['scrust_nounbox', ['--no-unbox']]]) {
            const r = spawnSync(path.join(ROOT, 'target/debug/scrust'), ['run', file, '--headless', ...extra, '--deterministic', '--frames', '3', '--', '--bench'], {encoding: 'utf8'});
            const m = /elapsed: ([0-9.]+) ms/.exec(r.stderr || '');
            row[key] = m ? Number(m[1]) : NaN;
        }
    }
    const pad = (s, n) => String(s).padStart(n);
    console.log(`${'project'.padEnd(10)}${pad('scratch-vm', 12)}${pad('TurboWarp', 12)}${pad('scrust(box)', 13)}${pad('scrust', 10)}${pad('vs TW', 9)}`);
    for (const r of rows) {
        console.log(`${r.project.padEnd(10)}${pad(r.vanilla.toFixed(1), 12)}${pad(r.tw.toFixed(1), 12)}${pad(r.scrust_nounbox.toFixed(1), 13)}${pad(r.scrust.toFixed(1), 10)}${pad((r.tw / r.scrust).toFixed(1) + 'x', 9)}`);
    }
}
