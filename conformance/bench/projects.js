// Benchmark projects (CPU-bound, warp procedures). `node projects.js` writes
// them to ../out/bench_*.json.
const fs = require('fs');
const path = require('path');
const {Project, b} = require('../harness/sb3');

function sieve (n) {
    const p = new Project();
    const s = p.sprite('S');
    const flags = p.stage.l('flags', []);
    const i = p.stage.v('i', 0); const j = p.stage.v('j', 0); const count = p.stage.v('count', 0); const done = p.stage.v('done', 0);
    s.define('sieve %s', ['n'], {warp: true}, [
        b.listClear(flags), b.repeat(b.arg('n'), [b.listAdd(flags, 1)]),
        b.set(i, 2),
        b.until(b.gt(b.mul(i, i), b.arg('n')), [
            b.if(b.eq(b.item(flags, i), 1), [b.set(j, b.mul(i, i)), b.until(b.gt(j, b.arg('n')), [b.listReplace(flags, j, 0), b.change(j, i)])]),
            b.change(i, 1)]),
        b.set(count, 0), b.set(i, 2),
        b.until(b.gt(i, b.arg('n')), [b.if(b.eq(b.item(flags, i), 1), [b.change(count, 1)]), b.change(i, 1)])
    ]);
    s.script(b.whenFlag(), [b.repeat(20, [b.call(s, 'sieve %s', [n])]), b.set(done, 1)]);
    return p;
}

function mandel (size) {
    const p = new Project();
    const s = p.sprite('S');
    const [px, py, x, y, x0, y0, it, total, t, done] = ['px', 'py', 'x', 'y', 'x0', 'y0', 'it', 'total', 't', 'done'].map(n => p.stage.v(n, 0));
    s.define('mandel %s', ['size'], {warp: true}, [
        b.set(total, 0), b.set(py, 0),
        b.repeat(b.arg('size'), [
            b.set(px, 0),
            b.repeat(b.arg('size'), [
                b.set(x0, b.sub(b.div(b.mul(px, 3.5), b.arg('size')), 2.5)), b.set(y0, b.sub(b.div(b.mul(py, 2), b.arg('size')), 1)),
                b.set(x, 0), b.set(y, 0), b.set(it, 0),
                b.until(b.or(b.gt(b.add(b.mul(x, x), b.mul(y, y)), 4), b.gt(it, 199)), [
                    b.set(t, b.add(b.sub(b.mul(x, x), b.mul(y, y)), x0)), b.set(y, b.add(b.mul(b.mul(2, x), y), y0)), b.set(x, t), b.change(it, 1)]),
                b.change(total, it), b.change(px, 1)]),
            b.change(py, 1)])
    ]);
    s.script(b.whenFlag(), [b.call(s, 'mandel %s', [size]), b.set(done, 1)]);
    return p;
}

function strings (n) {
    const p = new Project();
    const s = p.sprite('S');
    const [str, i, acc, done] = ['str', 'i', 'acc', 'done'].map(v => p.stage.v(v, 0));
    s.define('strings %s', ['n'], {warp: true}, [
        b.set(str, ''), b.set(i, 0),
        b.repeat(b.arg('n'), [b.set(str, b.join(str, b.letter(b.add(b.mod(i, 26), 1), 'abcdefghijklmnopqrstuvwxyz'))), b.change(i, 1)]),
        b.set(acc, 0), b.set(i, 1),
        b.repeat(b.arg('n'), [b.if(b.eq(b.letter(i, str), 'e'), [b.change(acc, 1)]), b.change(i, 1)]),
        b.set(acc, b.join(acc, b.join('/', b.length(str))))
    ]);
    s.script(b.whenFlag(), [b.call(s, 'strings %s', [n]), b.set(done, 1)]);
    return p;
}

function collatz (n) {
    const p = new Project();
    const s = p.sprite('S');
    const [k, c, best, steps, done] = ['k', 'c', 'best', 'steps', 'done'].map(v => p.stage.v(v, 0));
    s.define('collatz %s', ['n'], {warp: true}, [
        b.set(k, 1), b.set(best, 0),
        b.repeat(b.arg('n'), [
            b.set(c, k), b.set(steps, 0),
            b.until(b.eq(c, 1), [b.ifElse(b.eq(b.mod(c, 2), 0), [b.set(c, b.div(c, 2))], [b.set(c, b.add(b.mul(c, 3), 1))]), b.change(steps, 1)]),
            b.if(b.gt(steps, best), [b.set(best, steps)]), b.change(k, 1)])
    ]);
    s.script(b.whenFlag(), [b.call(s, 'collatz %s', [n]), b.set(done, 1)]);
    return p;
}

const all = {sieve: sieve(200000), mandel: mandel(300), strings: strings(300000), collatz: collatz(300000)};
module.exports = all;
if (require.main === module) {
    const out = path.join(__dirname, '..', 'out');
    fs.mkdirSync(out, {recursive: true});
    for (const [name, p] of Object.entries(all)) fs.writeFileSync(path.join(out, `bench_${name}.json`), p.json());
}
