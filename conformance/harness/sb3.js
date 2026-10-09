// Tiny DSL for synthesising Scratch 3 projects (project.json) in tests.
//
//   const {Project, b} = require('./sb3');
//   const p = new Project();
//   const s = p.sprite('Sprite1');
//   const score = p.stage.v('score', '0');
//   s.script(b.whenFlag(), [b.set(score, 1), b.repeat(10, [b.change(score, 1)])]);
//   p.json()
//
// Literals: JS strings become text primitives, JS numbers become the
// *string* form the Scratch editor writes ([4, "10"]). Use b.raw(10) for a
// genuine JSON number.

const EMPTY_SVG_MD5 = 'cd21514d0531fdffb22204e0ec5ed84a';

class Target {
    constructor (project, name, isStage) {
        this.project = project;
        this.name = name;
        this.isStage = isStage;
        this.blocks = {};
        this.variables = {};
        this.lists = {};
        this.n = 0;
        this.costumes = [{name: isStage ? 'backdrop1' : 'costume1'}];
        this.sounds = [];
        this.props = {};
    }
    nextId (p = 'b') { return `${this.name.replace(/[^A-Za-z0-9]/g, '')}_${p}${this.n++}`; }
    v (name, value = 0, id) {
        id = id || `v_${this.name}_${name}`;
        this.variables[id] = [name, value];
        return {kind: 'var', name, id};
    }
    l (name, items = [], id) {
        id = id || `l_${this.name}_${name}`;
        this.lists[id] = [name, items];
        return {kind: 'list', name, id};
    }
    // costume('name')                      -> empty placeholder costume
    // costume('name', svgText, [cx, cy])   -> real SVG asset (first call replaces the placeholder)
    costume (name, svg, center) {
        const c = {name};
        if (svg) {
            c.svg = svg;
            c.center = center || [0, 0];
            c.md5 = require('crypto').createHash('md5').update(svg).digest('hex');
            this.project.assets[`${c.md5}.svg`] = Buffer.from(svg);
            if (this.costumes.length === 1 && !this.costumes[0].svg && !this._customCostumes) this.costumes = [];
            this._customCostumes = true;
        }
        this.costumes.push(c);
        return this;
    }
    // sound('name', wavBuffer)
    sound (name, wav) {
        const md5 = require('crypto').createHash('md5').update(wav).digest('hex');
        this.project.assets[`${md5}.wav`] = wav;
        this.sounds.push({name, assetId: md5, dataFormat: 'wav', format: '', rate: 22050, sampleCount: Math.floor((wav.length - 44) / 2), md5ext: `${md5}.wav`});
        return this;
    }
    set (props) { Object.assign(this.props, props); return this; }

    // Compile an input value. Returns the sb3 input array or undefined.
    _input (parentId, value) {
        if (value === null || typeof value === 'undefined') return undefined;
        if (Array.isArray(value)) {
            const first = this._chain(value, parentId);
            return first ? [2, first] : [1, null];
        }
        if (typeof value === 'number') return [1, [4, String(value)]];
        if (typeof value === 'string') return [1, [10, value]];
        if (typeof value === 'boolean') return [1, [10, String(value)]];
        if (value.raw !== undefined) return [1, [typeof value.raw === 'number' ? 4 : 10, value.raw]];
        if (value.kind === 'var') return [3, [12, value.name, value.id], [10, '']];
        if (value.kind === 'list') return [3, [13, value.name, value.id], [10, '']];
        if (value.color !== undefined) return [1, [9, value.color]];
        if (value.bcast !== undefined) return [1, [11, value.bcast, this.project.broadcast(value.bcast)]];
        if (value.opcode) {
            const id = this._block(value, parentId, false);
            return value.shadow ? [1, id] : [3, id, [10, '']];
        }
        throw new Error(`bad input ${JSON.stringify(value)}`);
    }
    _block (desc, parentId, topLevel) {
        const id = desc.id || this.nextId();
        const blk = {opcode: desc.opcode, next: null, parent: parentId || null, inputs: {}, fields: {}, shadow: !!desc.shadow, topLevel: !!topLevel};
        if (topLevel) { blk.x = 0; blk.y = 0; }
        this.blocks[id] = blk;
        for (const [name, value] of Object.entries(desc.inputs || {})) {
            const inp = this._input(id, value);
            if (inp !== undefined) blk.inputs[name] = inp;
        }
        for (const [name, value] of Object.entries(desc.fields || {})) {
            if (value && value.kind) blk.fields[name] = [value.name, value.id];
            else if (value && value.bcast !== undefined) blk.fields[name] = [value.bcast, this.project.broadcast(value.bcast)];
            else blk.fields[name] = Array.isArray(value) ? value : [value, null];
        }
        if (desc.mutation) blk.mutation = Object.assign({tagName: 'mutation', children: []}, desc.mutation);
        return id;
    }
    _chain (stmts, parentId) {
        let first = null;
        let prev = null;
        for (const s of stmts) {
            if (!s) continue;
            const id = this._block(s, prev || parentId, false);
            if (prev) this.blocks[prev].next = id; else first = id;
            prev = id;
        }
        return first;
    }
    script (hat, body) {
        const id = this._block(hat, null, true);
        const first = this._chain(body, id);
        this.blocks[id].next = first;
        return this;
    }
    // define('foo %s %b', ['a', 'flag'], {warp: true}, body)
    define (proccode, argNames, opts, body) {
        opts = opts || {};
        const ids = argNames.map((_, i) => `${proccode.replace(/[^A-Za-z0-9]/g, '')}_arg${i}`);
        const kinds = (proccode.match(/%[sbn]/g) || []);
        const defaults = opts.defaults || kinds.map(k => (k === '%b' ? 'false' : ''));
        const protoId = this.nextId('proto');
        const defId = this.nextId('def');
        this.blocks[defId] = {opcode: 'procedures_definition', next: null, parent: null, inputs: {custom_block: [1, protoId]}, fields: {}, shadow: false, topLevel: true, x: 0, y: 0};
        const protoInputs = {};
        argNames.forEach((n, i) => {
            const rid = this.nextId('argrep');
            this.blocks[rid] = {opcode: kinds[i] === '%b' ? 'argument_reporter_boolean' : 'argument_reporter_string_number', next: null, parent: protoId, inputs: {}, fields: {VALUE: [n, null]}, shadow: true, topLevel: false};
            protoInputs[ids[i]] = [1, rid];
        });
        this.blocks[protoId] = {
            opcode: 'procedures_prototype', next: null, parent: defId, inputs: protoInputs, fields: {}, shadow: true, topLevel: false,
            mutation: {tagName: 'mutation', children: [], proccode, argumentids: JSON.stringify(ids), argumentnames: JSON.stringify(argNames), argumentdefaults: JSON.stringify(defaults), warp: String(!!opts.warp)}
        };
        this.blocks[defId].next = this._chain(body, defId);
        this.project.procs[`${this.name}/${proccode}`] = {ids, warp: !!opts.warp};
        return this;
    }
    toJSON (layer) {
        const costumes = this.costumes.map(c => ({name: c.name, bitmapResolution: 1, dataFormat: 'svg', assetId: c.md5 || EMPTY_SVG_MD5, md5ext: `${c.md5 || EMPTY_SVG_MD5}.svg`, rotationCenterX: c.center ? c.center[0] : 0, rotationCenterY: c.center ? c.center[1] : 0}));
        const base = {isStage: this.isStage, name: this.name, variables: this.variables, lists: this.lists, broadcasts: this.isStage ? this.project.broadcasts : {}, blocks: this.blocks, comments: {}, currentCostume: 0, costumes, sounds: this.sounds, volume: 100, layerOrder: layer};
        if (this.isStage) return Object.assign(base, {tempo: 60, videoTransparency: 50, videoState: 'on', textToSpeechLanguage: null}, this.props);
        return Object.assign(base, {visible: true, x: 0, y: 0, size: 100, direction: 90, draggable: false, rotationStyle: 'all around'}, this.props);
    }
}

class Project {
    constructor () {
        this.broadcasts = {};
        this.procs = {};
        this.assets = {};
        this.stage = new Target(this, 'Stage', true);
        this.sprites = [];
        this.monitors = [];
    }
    sprite (name) {
        const t = new Target(this, name, false);
        this.sprites.push(t);
        return t;
    }
    broadcast (name) {
        for (const [id, n] of Object.entries(this.broadcasts)) if (n === name) return id;
        const id = `bc_${Object.keys(this.broadcasts).length}`;
        this.broadcasts[id] = name;
        return id;
    }
    // A complete .sb3 archive (Promise<Buffer>).
    sb3 () {
        const JSZip = require('../vendor/scratch-vm/node_modules/jszip');
        const zip = new JSZip();
        zip.file('project.json', this.json());
        const EMPTY = '<svg version="1.1" width="2" height="2" viewBox="-1 -1 2 2" xmlns="http://www.w3.org/2000/svg"></svg>';
        zip.file(`${EMPTY_SVG_MD5}.svg`, EMPTY);
        for (const [name, data] of Object.entries(this.assets)) zip.file(name, data);
        return zip.generateAsync({type: 'nodebuffer', compression: 'DEFLATE'});
    }
    json () {
        const targets = [this.stage.toJSON(0), ...this.sprites.map((s, i) => s.toJSON(i + 1))];
        return JSON.stringify({targets, monitors: this.monitors, extensions: this.extensions || [], meta: {semver: '3.0.0', vm: '0.2.0', agent: 'scrust-conformance'}});
    }
}

// ---- block constructors -----------------------------------------------------
const B = (opcode, inputs, fields, extra) => Object.assign({opcode, inputs: inputs || {}, fields: fields || {}}, extra || {});
const bin = (opcode, a, b2, n1 = 'NUM1', n2 = 'NUM2') => B(opcode, {[n1]: a, [n2]: b2});
const b = {
    raw: v => ({raw: v}),
    // hats
    whenFlag: () => B('event_whenflagclicked'),
    whenKey: k => B('event_whenkeypressed', {}, {KEY_OPTION: k}),
    whenReceive: name => B('event_whenbroadcastreceived', {}, {BROADCAST_OPTION: {bcast: name}}),
    whenClone: () => B('control_start_as_clone'),
    whenBackdrop: name => B('event_whenbackdropswitchesto', {}, {BACKDROP: name}),
    whenGreater: (menu, v) => B('event_whengreaterthan', {VALUE: v}, {WHENGREATERTHANMENU: menu}),
    whenClicked: () => B('event_whenthisspriteclicked'),
    // operators
    add: (x, y) => bin('operator_add', x, y),
    sub: (x, y) => bin('operator_subtract', x, y),
    mul: (x, y) => bin('operator_multiply', x, y),
    div: (x, y) => bin('operator_divide', x, y),
    mod: (x, y) => bin('operator_mod', x, y),
    random: (x, y) => bin('operator_random', x, y, 'FROM', 'TO'),
    lt: (x, y) => bin('operator_lt', x, y, 'OPERAND1', 'OPERAND2'),
    eq: (x, y) => bin('operator_equals', x, y, 'OPERAND1', 'OPERAND2'),
    gt: (x, y) => bin('operator_gt', x, y, 'OPERAND1', 'OPERAND2'),
    and: (x, y) => bin('operator_and', x, y, 'OPERAND1', 'OPERAND2'),
    or: (x, y) => bin('operator_or', x, y, 'OPERAND1', 'OPERAND2'),
    not: x => B('operator_not', {OPERAND: x}),
    join: (x, y) => bin('operator_join', x, y, 'STRING1', 'STRING2'),
    letter: (i, s) => B('operator_letter_of', {LETTER: i, STRING: s}),
    length: s => B('operator_length', {STRING: s}),
    contains: (x, y) => bin('operator_contains', x, y, 'STRING1', 'STRING2'),
    round: x => B('operator_round', {NUM: x}),
    mathop: (op, x) => B('operator_mathop', {NUM: x}, {OPERATOR: op}),
    // data
    set: (v, x) => B('data_setvariableto', {VALUE: x}, {VARIABLE: v}),
    change: (v, x) => B('data_changevariableby', {VALUE: x}, {VARIABLE: v}),
    listAdd: (l, x) => B('data_addtolist', {ITEM: x}, {LIST: l}),
    listDelete: (l, i) => B('data_deleteoflist', {INDEX: i}, {LIST: l}),
    listClear: l => B('data_deletealloflist', {}, {LIST: l}),
    listInsert: (l, i, x) => B('data_insertatlist', {ITEM: x, INDEX: i}, {LIST: l}),
    listReplace: (l, i, x) => B('data_replaceitemoflist', {INDEX: i, ITEM: x}, {LIST: l}),
    item: (l, i) => B('data_itemoflist', {INDEX: i}, {LIST: l}),
    itemNum: (l, x) => B('data_itemnumoflist', {ITEM: x}, {LIST: l}),
    listLen: l => B('data_lengthoflist', {}, {LIST: l}),
    listHas: (l, x) => B('data_listcontainsitem', {ITEM: x}, {LIST: l}),
    // control
    repeat: (n, body) => B('control_repeat', {TIMES: n, SUBSTACK: body}),
    forever: body => B('control_forever', {SUBSTACK: body}),
    if: (c, body) => B('control_if', {CONDITION: c, SUBSTACK: body}),
    ifElse: (c, x, y) => B('control_if_else', {CONDITION: c, SUBSTACK: x, SUBSTACK2: y}),
    until: (c, body) => B('control_repeat_until', {CONDITION: c, SUBSTACK: body}),
    while: (c, body) => B('control_while', {CONDITION: c, SUBSTACK: body}),
    forEach: (v, n, body) => B('control_for_each', {VALUE: n, SUBSTACK: body}, {VARIABLE: v}),
    wait: s => B('control_wait', {DURATION: s}),
    waitUntil: c => B('control_wait_until', {CONDITION: c}),
    stop: what => B('control_stop', {}, {STOP_OPTION: what}, {mutation: {hasnext: String(what === 'other scripts in sprite')}}),
    clone: (what = '_myself_') => B('control_create_clone_of', {CLONE_OPTION: B('control_create_clone_of_menu', {}, {CLONE_OPTION: what}, {shadow: true})}),
    deleteClone: () => B('control_delete_this_clone'),
    // events
    broadcast: name => B('event_broadcast', {BROADCAST_INPUT: typeof name === 'string' ? {bcast: name} : name}),
    broadcastWait: name => B('event_broadcastandwait', {BROADCAST_INPUT: typeof name === 'string' ? {bcast: name} : name}),
    // looks / motion / sensing
    say: x => B('looks_say', {MESSAGE: x}),
    think: x => B('looks_think', {MESSAGE: x}),
    sayFor: (x, s) => B('looks_sayforsecs', {MESSAGE: x, SECS: s}),
    show: () => B('looks_show'),
    hide: () => B('looks_hide'),
    switchCostume: x => B('looks_switchcostumeto', {COSTUME: x}),
    nextCostume: () => B('looks_nextcostume'),
    switchBackdrop: x => B('looks_switchbackdropto', {BACKDROP: x}),
    switchBackdropWait: x => B('looks_switchbackdroptoandwait', {BACKDROP: x}),
    costumeNum: () => B('looks_costumenumbername', {}, {NUMBER_NAME: 'number'}),
    costumeName: () => B('looks_costumenumbername', {}, {NUMBER_NAME: 'name'}),
    backdropNum: () => B('looks_backdropnumbername', {}, {NUMBER_NAME: 'number'}),
    front: () => B('looks_gotofrontback', {}, {FRONT_BACK: 'front'}),
    back: () => B('looks_gotofrontback', {}, {FRONT_BACK: 'back'}),
    layers: (dir, n) => B('looks_goforwardbackwardlayers', {NUM: n}, {FORWARD_BACKWARD: dir}),
    setEffect: (e, v) => B('looks_seteffectto', {VALUE: v}, {EFFECT: e}),
    changeEffect: (e, v) => B('looks_changeeffectby', {CHANGE: v}, {EFFECT: e}),
    setSize: v => B('looks_setsizeto', {SIZE: v}),
    move: n => B('motion_movesteps', {STEPS: n}),
    turnRight: n => B('motion_turnright', {DEGREES: n}),
    turnLeft: n => B('motion_turnleft', {DEGREES: n}),
    gotoXY: (x, y) => B('motion_gotoxy', {X: x, Y: y}),
    goTo: what => B('motion_goto', {TO: B('motion_goto_menu', {}, {TO: what}, {shadow: true})}),
    glide: (s, x, y) => B('motion_glidesecstoxy', {SECS: s, X: x, Y: y}),
    point: d => B('motion_pointindirection', {DIRECTION: d}),
    pointTowards: what => B('motion_pointtowards', {TOWARDS: B('motion_pointtowards_menu', {}, {TOWARDS: what}, {shadow: true})}),
    changeX: n => B('motion_changexby', {DX: n}),
    changeY: n => B('motion_changeyby', {DY: n}),
    setX: n => B('motion_setx', {X: n}),
    setY: n => B('motion_sety', {Y: n}),
    xpos: () => B('motion_xposition'),
    ypos: () => B('motion_yposition'),
    dir: () => B('motion_direction'),
    timer: () => B('sensing_timer'),
    resetTimer: () => B('sensing_resettimer'),
    answer: () => B('sensing_answer'),
    ask: q => B('sensing_askandwait', {QUESTION: q}),
    of: (prop, obj) => B('sensing_of', {OBJECT: typeof obj === 'string' ? B('sensing_of_object_menu', {}, {OBJECT: obj}, {shadow: true}) : obj}, {PROPERTY: prop}),
    distanceTo: what => B('sensing_distanceto', {DISTANCETOMENU: B('sensing_distancetomenu', {}, {DISTANCETOMENU: what}, {shadow: true})}),
    // sound
    playSound: name => B('sound_play', {SOUND_MENU: B('sound_sounds_menu', {}, {SOUND_MENU: name}, {shadow: true})}),
    playSoundWait: name => B('sound_playuntildone', {SOUND_MENU: B('sound_sounds_menu', {}, {SOUND_MENU: name}, {shadow: true})}),
    setVolume: v => B('sound_setvolumeto', {VOLUME: v}),
    setPitch: v => B('sound_seteffectto', {VALUE: v}, {EFFECT: 'PITCH'}),
    // pen
    penClear: () => B('pen_clear'),
    penDown: () => B('pen_penDown'),
    penUp: () => B('pen_penUp'),
    penStamp: () => B('pen_stamp'),
    penColor: c => B('pen_setPenColorToColor', {COLOR: typeof c === 'string' ? {color: c} : c}),
    penSize: n => B('pen_setPenSizeTo', {SIZE: n}),
    penParam: (param, v) => B('pen_setPenColorParamTo', {COLOR_PARAM: B('pen_menu_colorParam', {}, {colorParam: param}, {shadow: true}), VALUE: v}),
    penChangeParam: (param, v) => B('pen_changePenColorParamBy', {COLOR_PARAM: B('pen_menu_colorParam', {}, {colorParam: param}, {shadow: true}), VALUE: v}),
    touching: what => B('sensing_touchingobject', {TOUCHINGOBJECTMENU: B('sensing_touchingobjectmenu', {}, {TOUCHINGOBJECTMENU: what}, {shadow: true})}),
    touchingColor: c => B('sensing_touchingcolor', {COLOR: {color: c}}),
    bounce: () => B('motion_ifonedgebounce'),
    showVar: v => B('data_showvariable', {}, {VARIABLE: v}),
    mouseX: () => B('sensing_mousex'),
    mouseY: () => B('sensing_mousey'),
    keyPressed: k => B('sensing_keypressed', {KEY_OPTION: B('sensing_keyoptions', {}, {KEY_OPTION: k}, {shadow: true})}),
    // procedures
    call: (target, proccode, args = []) => {
        const info = target.project.procs[`${target.name}/${proccode}`];
        const ids = info ? info.ids : args.map((_, i) => `${proccode.replace(/[^A-Za-z0-9]/g, '')}_arg${i}`);
        const inputs = {};
        args.forEach((a, i) => { inputs[ids[i]] = a; });
        return B('procedures_call', inputs, {}, {mutation: {proccode, argumentids: JSON.stringify(ids), warp: String(info ? info.warp : false)}});
    },
    arg: name => B('argument_reporter_string_number', {}, {VALUE: name}),
    argBool: name => B('argument_reporter_boolean', {}, {VALUE: name})
};

// 16-bit mono PCM WAV with a decaying sine tone.
function toneWav (freq, seconds, rate = 22050) {
    const n = Math.floor(seconds * rate);
    const buf = Buffer.alloc(44 + n * 2);
    buf.write('RIFF', 0); buf.writeUInt32LE(36 + n * 2, 4); buf.write('WAVE', 8); buf.write('fmt ', 12);
    buf.writeUInt32LE(16, 16); buf.writeUInt16LE(1, 20); buf.writeUInt16LE(1, 22); buf.writeUInt32LE(rate, 24);
    buf.writeUInt32LE(rate * 2, 28); buf.writeUInt16LE(2, 32); buf.writeUInt16LE(16, 34); buf.write('data', 36); buf.writeUInt32LE(n * 2, 40);
    for (let i = 0; i < n; i++) buf.writeInt16LE(Math.round(Math.sin(2 * Math.PI * freq * i / rate) * 12000 * (1 - i / n)), 44 + i * 2);
    return buf;
}

module.exports = {Project, Target, b, B, toneWav};
