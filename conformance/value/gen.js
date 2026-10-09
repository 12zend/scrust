// Generates random cross-check cases for scrust-value on stdout.
//   node gen.js <count> [seed] | cargo run --release -p scrust-value --example crosscheck
// Kinds: see crates/scrust-value/examples/crosscheck.rs
const count = Number(process.argv[2] || 1e6);
let seed = BigInt(process.argv[3] || 12345);
const M = (1n << 64n) - 1n;
function next () { // xorshift64*
    seed ^= seed >> 12n; seed ^= (seed << 25n) & M; seed ^= seed >> 27n;
    return (seed * 0x2545F4914F6CDD1Dn) & M;
}
const buf = new DataView(new ArrayBuffer(8));
const bitsOf = x => { buf.setFloat64(0, x); return buf.getBigUint64(0).toString(16); };
const fromBits = b => { buf.setBigUint64(0, b); return buf.getFloat64(0); };
const r32 = () => Number(next() >> 32n);
const ri = n => r32() % n;
const hex = s => { let o = ''; for (let i = 0; i < s.length; i++) o += s.charCodeAt(i).toString(16).padStart(4, '0'); return o || '-'; };

function randNum () {
    switch (ri(8)) {
    case 0: return fromBits(next());                      // any bit pattern
    case 1: return (ri(2000001) - 1000000) / 1000;        // short decimals
    case 2: return ri(2001) - 1000;                       // small ints
    case 3: return (r32() / 4294967296) * Math.pow(10, ri(50) - 25);
    case 4: return Math.pow(2, ri(120) - 60) * (ri(1000) + 1) * (ri(2) ? 1 : -1);
    case 5: return (ri(100000) + 0.5) / Math.pow(2, ri(20)); // toFixed ties
    case 6: return Number((r32() / 4294967296).toPrecision(1 + ri(17)));
    default: return (r32() * 4294967296 + r32()) * (ri(2) ? 1 : -1) / (ri(3) ? 1 : 1e10);
    }
}
const WS = [' ', '\t', '\n', ' ', '﻿', '　', ' ', '᠎', '​', '\u0085'];
const PIECES = ['0', '1', '9', '.', 'e', 'E', '+', '-', 'x', 'X', 'b', 'o', 'Infinity', 'infinity', 'NaN', 'a', 'f', '_', ',', '00', '5', '7', ' '];
function randStr () {
    switch (ri(7)) {
    case 0: return String(randNum());
    case 1: return WS[ri(WS.length)] + String(randNum()) + WS[ri(WS.length)];
    case 2: { let s = ''; const n = ri(8); for (let i = 0; i < n; i++) s += PIECES[ri(PIECES.length)]; return s; }
    case 3: return randNum().toExponential(ri(20));
    case 4: return (ri(2) ? '0x' : ri(2) ? '0b' : '0o') + next().toString(ri(2) ? 16 : ri(2) ? 2 : 8) + (ri(4) ? '' : next().toString(16));
    case 5: return randNum().toFixed(ri(30));
    default: { let s = String(randNum()); const i = ri(s.length + 1); return s.slice(0, i) + PIECES[ri(PIECES.length)] + s.slice(i); }
    }
}
function randText () {
    let s = ''; const n = ri(12);
    for (let i = 0; i < n; i++) {
        const k = ri(10);
        if (k < 4) s += String.fromCharCode(32 + ri(95));
        else if (k < 6) s += String.fromCharCode(0xa0 + ri(0x60));
        else if (k < 8) s += String.fromCharCode(0x100 + ri(0x500));
        else if (k < 9) s += String.fromCharCode(ri(0x10000));
        else s += ['Σ', 'İ', 'ΑΣ ', 'ǅ', 'ẞ', 'K', '😀', 'Ⅷ', '𐐀'][ri(9)];
    }
    return s;
}
const OPS = ['abs', 'floor', 'ceiling', 'sqrt', 'sin', 'cos', 'tan', 'asin', 'acos', 'atan', 'ln', 'log', 'e ^', '10 ^'];
const MathUtil = { tan (angle) { angle = angle % 360; switch (angle) { case -270: case 90: return Infinity; case -90: case 270: return -Infinity; default: return parseFloat(Math.tan((Math.PI * angle) / 180).toFixed(10)); } } };
function mathop (operator, n) {
    switch (operator) {
    case 'abs': return Math.abs(n);
    case 'floor': return Math.floor(n);
    case 'ceiling': return Math.ceil(n);
    case 'sqrt': return Math.sqrt(n);
    case 'sin': return parseFloat(Math.sin((Math.PI * n) / 180).toFixed(10));
    case 'cos': return parseFloat(Math.cos((Math.PI * n) / 180).toFixed(10));
    case 'tan': return MathUtil.tan(n);
    case 'asin': return (Math.asin(n) * 180) / Math.PI;
    case 'acos': return (Math.acos(n) * 180) / Math.PI;
    case 'atan': return (Math.atan(n) * 180) / Math.PI;
    case 'ln': return Math.log(n);
    case 'log': return Math.log(n) / Math.LN10;
    case 'e ^': return Math.exp(n);
    case '10 ^': return Math.pow(10, n);
    }
    return 0;
}
const out = [];
const flush = () => { process.stdout.write(out.join('\n') + '\n'); out.length = 0; };
const only = process.argv[4];
for (let i = 0; i < count; i++) {
    const k = only ? -1 : ri(100);
    if (only === 'S' || (k >= 0 && k < 40)) { const s = randStr(); out.push(`S ${hex(s)} ${bitsOf(Number(s))}`); }
    else if (only === 'N' || (k >= 40 && k < 80)) { const x = randNum(); out.push(`N ${bitsOf(x)} ${String(x)}`); }
    else if (k < 86) { const x = randNum(); out.push(`F ${bitsOf(x)} ${bitsOf(parseFloat(x.toFixed(10)))}`); }
    else if (k < 90) { const s = randText(); out.push(`L ${hex(s)} ${hex(s.toLowerCase())}`); }
    else if (k < 93) { const x = randNum(); out.push(`I ${bitsOf(x)} ${x === parseInt(x, 10) || Number.isNaN(x) ? 1 : 0}`); }
    else if (k < 95) { const x = ri(2) ? randNum() : (ri(2001) - 1000) / 2; out.push(`R ${bitsOf(x)} ${bitsOf(Math.round(x))}`); }
    else { const op = OPS[ri(OPS.length)]; let x = randNum(); if (ri(2)) x = (ri(7200001) - 3600000) / 10000; out.push(`M ${op.replace(' ', '_')} ${bitsOf(x)} ${bitsOf(mathop(op, x))}`); }
    if (out.length >= 20000) flush();
}
flush();
