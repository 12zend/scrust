// Value semantics: PRODUCT.md §15.3 table and the cast corollaries of §5-§7.
const {Project, b} = require('../harness/sb3');

// Each expression is evaluated twice: appended raw to `out`, and through a
// variable (exercising storage typing).
function exprProject (exprs) {
    const p = new Project();
    const s = p.sprite('Sprite1');
    const out = p.stage.l('out', []);
    const tmp = p.stage.v('tmp', 0);
    const body = [];
    for (const e of exprs) body.push(b.listAdd(out, e));
    for (const e of exprs.slice(0, 40)) { body.push(b.set(tmp, e)); body.push(b.listAdd(out, tmp)); }
    s.script(b.whenFlag(), body);
    return p;
}

module.exports = () => {
    const cases = [];
    const nan = b.div(0, 0);
    const table = [
        b.add('', 1), b.mul('abc', 2), b.add(' 12 ', 0), b.add('0x10', 0), b.add('-0x10', 0), b.add('1_0', 0),
        b.add('Infinity', 0), b.add('infinity', 0), nan, b.mul(nan, 1), b.div(1, 0), b.add(0.1, 0.2),
        b.join(b.mul('1e20', 10), ''), b.join(b.mul(0, -1), ''), b.div(1, b.mul(0, -1)), b.round(-2.5), b.round(2.5),
        b.mod(-1, 3), b.mod(5, -3), b.mathop('log', 1000), b.mathop('sin', 180), b.mathop('cos', 90), b.mathop('tan', 90),
        b.mathop('tan', -90), b.length('😀'), b.letter(1.9, 'abc'), b.letter(0.5, 'abc'), b.contains('abc', ''),
        b.contains('Apple', 'PP'), b.eq(nan, nan), b.eq('', 0), b.eq('1.0', 1), b.eq('abc', 'ABC'), b.lt('10', '9'),
        b.lt('10', '9a'), b.eq(b.eq(1, 1), 'TRUE'), b.not('false'), b.not('0.0'),
        // more corollaries
        b.eq(' ', 0), b.eq('0', ''), b.eq('1e3', 1000), b.eq('0x10', 16), b.eq(b.eq(1, 1), 1), b.eq(b.div(1, 0), 'Infinity'),
        b.mul(0, -1), b.sub(b.mul(0, -1), 0), b.round(-0.4), b.round('0.49999999999999994'), b.mod(5, 0), b.mod(-5, 3), b.mod(5.5, 2),
        b.mathop('sqrt', -1), b.mathop('abs', -3), b.mathop('floor', -1.5), b.mathop('ceiling', -1.5), b.mathop('ln', 0),
        b.mathop('e ^', 1), b.mathop('10 ^', 3), b.mathop('10 ^', -2), b.mathop('asin', 2), b.mathop('atan', 1), b.mathop('sin', 30),
        b.mathop('cos', 60), b.mathop('tan', 45), b.mathop('tan', 270), b.mathop('tan', 450), b.mathop('sin', '1e30'),
        b.join(1, 2), b.join(b.add(1, 2), 'x'), b.join(b.div(1, 3), ''), b.join(b.mul('1e21', 1), ''), b.join(b.div(1, '1e7'), ''),
        b.join(b.div(1, '1e6'), ''), b.join(b.mul('123456789012345680000', 1), ''), b.join(b.eq(1, 1), b.eq(1, 2)),
        b.letter(1, '😀'), b.letter(2, '😀'), b.length(b.letter(1, '😀')), b.letter(0, 'abc'), b.letter(4, 'abc'), b.letter('x', 'abc'),
        b.length(12345), b.length(b.div(1, 3)), b.length(b.eq(1, 1)), b.contains('İstanbul', 'i̇'), b.contains(123, 2),
        b.add('1.', '.5'), b.add('+3', '1E+3'), b.add('\t7\n', 0), b.add(' 5', 0), b.add('0b101', '0o17'), b.add('0X1f', 0),
        b.add('-Infinity', 0), b.add('NaN', 1), b.add('true', 1), b.add(b.eq(1, 1), 1), b.add('1 2', 0), b.add('.', 0), b.add('e5', 0),
        b.add('١٢', 0), b.sub('9007199254740993', 0), b.add('0x20000000000001', 0), b.add('5e-324', 0), b.add('1e400', 0),
        b.lt('a', 'B'), b.gt('b', 'A'), b.lt('', ' '), b.eq('', ' '), b.lt(-1, ''), b.gt(1, 'a'), b.lt('Z', 'a'), b.eq('İ', 'i̇'),
        b.lt(b.div(-1, 0), b.div(1, 0)), b.eq(b.div(1, 0), b.div(1, 0)), b.gt(nan, 0), b.lt(nan, 0), b.eq(nan, 0), b.eq(nan, 'NaN'),
        b.eq('-0', 0), b.eq('0.0', 0), b.eq('00', 0), b.eq(' 0', 0), b.eq('true', b.eq(1, 1)), b.eq('false', b.eq(1, 2)), b.eq(0, b.eq(1, 2)),
        b.and('true', 'x'), b.and('0', 'x'), b.or('', 'false'), b.or('', 'FALSE '), b.not(''), b.not('0'), b.not(' 0'), b.not('00'),
        b.not('no'), b.not('False'), b.not(nan), b.not(b.mul(0, -1)), b.not(0.1), b.not(b.join('', '')),
        b.round('abc'), b.round(b.div(1, 0)), b.round(nan), b.round(-0.5), b.round(0.5), b.round(1.5), b.round('4503599627370497'),
        b.div(0, b.mul(0, -1)), b.div(-1, 0), b.sub(b.div(1, 0), b.div(1, 0)), b.mul(b.div(1, 0), 0), b.add('1e308', '1e308'),
        b.mathop('abs', nan), b.mathop('floor', b.div(1, 0)), b.mathop('ceiling', -0.2), b.mathop('sqrt', 2), b.mathop('log', 0),
        b.mathop('ln', -1), b.mathop('e ^', 1000), b.mathop('10 ^', 400), b.mathop('acos', 0.5), b.mathop('asin', 1)
    ];
    cases.push({name: 'table', project: exprProject(table), frames: 2});

    // Variables keep raw values; change coerces.
    {
        const p = new Project();
        const s = p.sprite('Sprite1');
        const out = p.stage.l('out', []);
        const a = p.stage.v('a', '007');
        const c = p.stage.v('c', 0);
        const d = s.v('local', 'x');
        const e = p.stage.v('e', b.raw ? 0 : 0);
        s.script(b.whenFlag(), [
            b.listAdd(out, a), b.set(a, '1.0'), b.listAdd(out, a), b.listAdd(out, b.join(a, '')), b.listAdd(out, b.length(a)),
            b.change(a, 1), b.listAdd(out, a), b.set(a, 'abc'), b.change(a, 1), b.listAdd(out, a),
            b.set(c, b.div(0, 0)), b.listAdd(out, c), b.listAdd(out, b.join(c, '')), b.change(c, 5), b.listAdd(out, c),
            b.set(c, b.mul(0, -1)), b.listAdd(out, c), b.listAdd(out, b.div(1, c)), b.set(c, '-0'), b.listAdd(out, b.div(1, c)),
            b.set(d, b.eq(1, 1)), b.listAdd(out, d), b.listAdd(out, b.add(d, 1)), b.listAdd(out, b.join(d, '!')),
            b.set(e, b.raw(10)), b.listAdd(out, e), b.set(e, b.raw('10')), b.listAdd(out, e), b.set(e, 10), b.listAdd(out, e),
            b.set(e, b.raw(1.5)), b.listAdd(out, e), b.set(e, b.raw(-0)), b.listAdd(out, e), b.listAdd(out, b.div(1, e))
        ]);
        cases.push({name: 'variables', project: p, frames: 2});
    }

    // Lists.
    {
        const p = new Project();
        const s = p.sprite('Sprite1');
        const out = p.stage.l('out', []);
        const l = p.stage.l('l', ['a', 'b']);
        const m = s.l('m', [4, '123', 'Apple', '', ' ', '0', 0]);
        const k = p.stage.l('k', [1, 2, 3]);
        const ch = p.stage.l('ch', ['a', 'b']);
        s.script(b.whenFlag(), [
            b.listAdd(out, l), b.listAdd(l, b.add(0, 1)), b.listAdd(out, l), b.listAdd(out, b.item(l, 1.9)), b.listAdd(out, b.item(l, 'last')),
            b.listAdd(out, b.item(l, 0)), b.listAdd(out, b.item(l, 4)), b.listAdd(out, b.item(l, 'abc')), b.listAdd(out, b.item(l, 'LAST')),
            b.listAdd(out, b.item(l, b.div(1, 0))), b.listAdd(out, b.item(l, '2')), b.listAdd(out, b.item(l, 'all')),
            b.listAdd(out, b.itemNum(m, 123)), b.listAdd(out, b.itemNum(m, 'apple')), b.listAdd(out, b.itemNum(m, '')), b.listAdd(out, b.itemNum(m, 0)),
            b.listAdd(out, b.itemNum(m, 'zzz')), b.listAdd(out, b.listHas(m, 'APPLE')), b.listHas(m, 4) && b.listAdd(out, b.listHas(m, '4.0')),
            b.listAdd(out, b.listHas(m, ' ')), b.listAdd(out, b.listHas(m, 'x')), b.listAdd(out, b.listLen(m)),
            b.listInsert(l, 1, 'first'), b.listInsert(l, 'last', 'L'), b.listInsert(l, 99, 'no'), b.listInsert(l, b.add(b.listLen(l), 1), 'end'),
            b.listAdd(out, l), b.listReplace(l, 2, b.mul(2, 3)), b.listReplace(l, 0, 'no'), b.listReplace(l, 'last', 'Z'), b.listAdd(out, l),
            b.listDelete(l, 1), b.listDelete(l, 'last'), b.listDelete(l, 99), b.listDelete(l, 'abc'), b.listAdd(out, l),
            b.listAdd(out, k), b.listAdd(out, ch), b.listAdd(ch, 'cd'), b.listAdd(out, ch), b.listDelete(ch, 'all'), b.listAdd(out, ch),
            b.listAdd(out, b.listLen(ch)), b.listAdd(out, b.item(ch, 'last')), b.listAdd(out, b.item(ch, 'any')),
            b.listAdd(k, '4'), b.listAdd(out, k), b.listClear(k), b.listAdd(k, '1'), b.listAdd(k, '2'), b.listAdd(out, k),
            b.listAdd(k, 3), b.listAdd(out, k), b.listAdd(out, b.item(k, 'random')), b.listAdd(out, b.item(k, 'any')),
            b.listAdd(out, b.add(b.item(k, 1), b.item(k, 9))), b.listAdd(out, b.join(b.item(k, 3), b.item(k, 9)))
        ].filter(Boolean));
        cases.push({name: 'lists', project: p, frames: 2});
    }

    // Random numbers: isInt on raw operands, evaluation order of the RNG.
    {
        const p = new Project();
        const s = p.sprite('Sprite1');
        const out = p.stage.l('out', []);
        const v = p.stage.v('v', '1.0');
        const n = p.stage.v('n', 3);
        s.script(b.whenFlag(), [
            b.listAdd(out, b.random(1, 1)), b.listAdd(out, b.random(1, 10)), b.listAdd(out, b.random('1.0', 2)), b.listAdd(out, b.random(10, 1)),
            b.listAdd(out, b.random(v, 5)), b.listAdd(out, b.random(n, 5)), b.listAdd(out, b.random(b.add(1, 0.5), 5)), b.listAdd(out, b.random(-5, 5)),
            b.listAdd(out, b.random('a', 'b')), b.listAdd(out, b.random('', 3)), b.listAdd(out, b.random(b.eq(1, 1), 4)), b.listAdd(out, b.random(0, '1e21')),
            b.listAdd(out, b.random(0.5, 0.5)), b.listAdd(out, b.random('5', '2.5')), b.listAdd(out, b.random(b.div(0, 0), 3)),
            b.listAdd(out, b.join(b.random(1, 100), b.join('/', b.random(1, 1000)))), b.listAdd(out, b.sub(b.random(1, 100), b.random(1, 1000))),
            b.set(n, b.random(1, 6)), b.listAdd(out, n), b.listAdd(out, b.and(b.gt(b.random(1, 10), 5), b.gt(b.random(1, 10), 5)))
        ]);
        cases.push({name: 'random', project: p, frames: 2, seed: 42});
    }
    return cases;
};
