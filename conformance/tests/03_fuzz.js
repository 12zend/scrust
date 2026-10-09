// Expression fuzzer (PRODUCT.md §15.2): random expression trees over the
// cast boundary dictionary, random variable / list statements.
// FUZZ_PROJECTS / FUZZ_SEED environment variables scale it up.
const {Project, b} = require('../harness/sb3');

function rng (seed) {
    let a = seed | 0;
    return () => { a = (a + 0x6D2B79F5) | 0; let t = Math.imul(a ^ (a >>> 15), 1 | a); t = (t + Math.imul(t ^ (t >>> 7), 61 | t)) ^ t; return ((t ^ (t >>> 14)) >>> 0) / 4294967296; };
}

const STR = ['', ' ', '\t', ' ', '0', '-0', '0.0', '00', '1.', '.5', '1e3', '1E+3', '0x1F', '0X1f', '0b101', '0o17', '-0x1', 'Infinity', '-Infinity', 'infinity',
    'NaN', 'true', 'FALSE', 'abc', 'ABC', 'İ', 'Σ', '😀', '\ud83d', '1', '2', '3', '10', '-1', '1.5', '2.5', '-2.5', '100', 'a', 'B', 'last', 'all', 'random', 'any',
    '9007199254740993', '1e21', '1e-7', '0.1', ' 5 ', 'Apple', 'apple pie', '1 2'];
const RAW = [0, 0.5, -0.5, 2.5, -2.5, 1e21, 1e-7, 2 ** 53, 2 ** 53 + 2, 5e-324, 1, 2, 3, -1, 10, 255, 0.1, 1e308];
const MATH = ['abs', 'floor', 'ceiling', 'sqrt', 'sin', 'cos', 'tan', 'asin', 'acos', 'atan', 'ln', 'log', 'e ^', '10 ^'];
// Functions whose last-ULP results legitimately differ between libm implementations.
const EXACT_MATH = ['abs', 'floor', 'ceiling', 'sqrt', 'sin', 'cos', 'tan'];

function build (seed, nStmts) {
    const r = rng(seed);
    const pick = xs => xs[Math.floor(r() * xs.length)];
    const p = new Project();
    const s = p.sprite('Sprite1');
    const out = p.stage.l('out', []);
    const vars = [];
    for (let i = 0; i < 6; i++) vars.push(p.stage.v(`v${i}`, r() < 0.5 ? pick(STR) : pick(RAW)));
    for (let i = 0; i < 3; i++) vars.push(s.v(`w${i}`, r() < 0.5 ? pick(STR) : pick(RAW)));
    const lists = [];
    for (let i = 0; i < 3; i++) {
        const items = [];
        const n = Math.floor(r() * 6);
        for (let k = 0; k < n; k++) items.push(r() < 0.5 ? pick(STR) : pick(RAW));
        lists.push(p.stage.l(`l${i}`, items));
    }
    const special = () => pick([b.div(0, 0), b.div(1, 0), b.div(-1, 0), b.mul(0, -1), b.eq(1, 1), b.eq(1, 2)]);
    const leaf = () => {
        const k = r();
        if (k < 0.3) return pick(STR);
        if (k < 0.45) return b.raw(pick(RAW));
        if (k < 0.55) return pick(RAW.filter(x => Number.isInteger(x) && Math.abs(x) < 1000));
        if (k < 0.75) return pick(vars);
        if (k < 0.85) return special();
        if (k < 0.92) return b.listLen(pick(lists));
        return pick(lists);
    };
    const expr = d => {
        if (d <= 0 || r() < 0.2) return leaf();
        const e = () => expr(d - 1);
        const k = Math.floor(r() * 26);
        switch (k) {
        case 0: return b.add(e(), e());
        case 1: return b.sub(e(), e());
        case 2: return b.mul(e(), e());
        case 3: return b.div(e(), e());
        case 4: return b.mod(e(), e());
        case 5: return b.round(e());
        case 6: return b.lt(e(), e());
        case 7: return b.eq(e(), e());
        case 8: return b.gt(e(), e());
        case 9: return b.and(e(), e());
        case 10: return b.or(e(), e());
        case 11: return b.not(e());
        case 12: return b.join(e(), e());
        case 13: return b.letter(e(), e());
        case 14: return b.length(e());
        case 15: return b.contains(e(), e());
        case 16: return b.mathop(pick(EXACT_MATH), e());
        case 17: return b.item(pick(lists), e());
        case 18: return b.itemNum(pick(lists), e());
        case 19: return b.listHas(pick(lists), e());
        case 20: return b.random(e(), e());
        case 21: return b.eq(e(), leaf());
        case 22: return b.lt(leaf(), e());
        case 23: return b.join(e(), leaf());
        case 24: return b.add(e(), leaf());
        default: return b.item(pick(lists), pick(['last', 'random', 'any', 1, 2, 3, '1.5', 0, 'all', 'abc']));
        }
    };
    const body = [];
    for (let i = 0; i < nStmts; i++) {
        const k = r();
        const e = expr(3);
        if (k < 0.45) body.push(b.listAdd(out, e));
        else if (k < 0.6) body.push(b.set(pick(vars), e));
        else if (k < 0.68) body.push(b.change(pick(vars), e));
        else if (k < 0.76) body.push(b.listAdd(pick(lists), e));
        else if (k < 0.82) body.push(b.listInsert(pick(lists), expr(1), e));
        else if (k < 0.88) body.push(b.listReplace(pick(lists), expr(1), e));
        else if (k < 0.92) body.push(b.listDelete(pick(lists), expr(1)));
        else if (k < 0.96) body.push(b.ifElse(e, [b.listAdd(out, 'T')], [b.listAdd(out, 'F')]));
        else body.push(b.say(e));
    }
    for (const v of vars) body.push(b.listAdd(out, v));
    for (const l of lists) body.push(b.listAdd(out, l));
    s.script(b.whenFlag(), body);
    return p;
}

const main = () => {
    const n = Number(process.env.FUZZ_PROJECTS || 6);
    const base = Number(process.env.FUZZ_SEED || 1000);
    const cases = [];
    for (let i = 0; i < n; i++) cases.push({name: `seed${base + i}`, project: build(base + i, 120), frames: 2, seed: base + i, maxPasses: 1});
    return cases;
};
module.exports = Object.assign(main, {build});
