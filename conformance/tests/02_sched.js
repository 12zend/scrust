// Scheduling: thread order, yields, broadcasts, clones, procedures, stops.
const {Project, b} = require('../harness/sb3');

module.exports = () => {
    const cases = [];
    const add = (name, build, extra = {}) => {
        const p = new Project();
        build(p);
        for (const maxPasses of extra.passes || [1, 3]) {
            cases.push(Object.assign({name: `${name}_p${maxPasses}`, project: p, frames: 25, maxPasses}, extra));
        }
    };

    add('order', p => {
        const log = p.stage.l('log', []);
        const a = p.sprite('A'); const bb = p.sprite('B'); const c = p.sprite('C');
        for (const [t, n] of [[p.stage, 'S'], [a, 'A'], [bb, 'B'], [c, 'C']]) {
            t.script(b.whenFlag(), [b.listAdd(log, `${n}1`), b.wait(0), b.listAdd(log, `${n}1b`)]);
            t.script(b.whenFlag(), [b.listAdd(log, `${n}2`), b.repeat(2, [b.listAdd(log, `${n}2r`)])]);
        }
    });

    add('loops', p => {
        const s = p.sprite('S');
        const x = p.stage.v('x', 0); const y = p.stage.v('y', 0); const z = p.stage.v('z', 0); const log = p.stage.l('log', []);
        s.script(b.whenFlag(), [b.forever([b.change(x, 1)])]);
        s.script(b.whenFlag(), [b.repeat(3, [b.repeat(2, [b.change(y, 1)]), b.listAdd(log, b.join(x, b.join(',', y)))])]);
        s.script(b.whenFlag(), [b.until(b.gt(z, 4), [b.change(z, 1), b.if(b.eq(b.mod(z, 2), 0), [b.listAdd(log, 'even')])]), b.listAdd(log, b.join('z done at x=', x))]);
        s.script(b.whenFlag(), [b.repeat(0, [b.listAdd(log, 'never')]), b.repeat(-3, [b.listAdd(log, 'never')]), b.repeat('abc', [b.listAdd(log, 'never')]),
            b.repeat(2.5, [b.listAdd(log, 'r2.5')]), b.repeat(2.4, [b.listAdd(log, 'r2.4')]), b.repeat(3, []), b.listAdd(log, b.join('after empty x=', x)),
            b.while(b.lt(y, 100), [b.change(y, 30)]), b.listAdd(log, b.join('while done x=', x))]);
        const i = p.stage.v('i', 0);
        s.script(b.whenFlag(), [b.forEach(i, 3, [b.listAdd(log, b.join('fe', i))]), b.forEach(i, '2.5', [b.listAdd(log, b.join('ff', i))]), b.listAdd(log, i)]);
    });

    add('wait', p => {
        const s = p.sprite('S');
        const x = p.stage.v('x', 0); const log = p.stage.l('log', []); const go = p.stage.v('go', 0);
        s.script(b.whenFlag(), [b.forever([b.change(x, 1)])]);
        s.script(b.whenFlag(), [b.wait(0), b.listAdd(log, b.join('w0 ', x)), b.wait(0.1), b.listAdd(log, b.join('w.1 ', x)), b.wait(-1), b.listAdd(log, b.join('w-1 ', x)),
            b.wait('abc'), b.listAdd(log, b.join('wabc ', x)), b.set(go, 1), b.wait(0.2), b.set(go, 2)]);
        s.script(b.whenFlag(), [b.waitUntil(b.eq(go, 1)), b.listAdd(log, b.join('go1 ', x)), b.waitUntil(b.eq(go, 2)), b.listAdd(log, b.join('go2 ', x)), b.waitUntil(b.eq(1, 1)), b.listAdd(log, 'imm')]);
        s.script(b.whenFlag(), [b.listAdd(log, b.timer()), b.wait(0.1), b.listAdd(log, b.timer()), b.resetTimer(), b.listAdd(log, b.timer()), b.wait(0.05), b.listAdd(log, b.timer())]);
    });

    add('broadcast', p => {
        const log = p.stage.l('log', []); const n = p.stage.v('n', 0);
        const a = p.sprite('A'); const c = p.sprite('B');
        a.script(b.whenFlag(), [b.broadcast('go'), b.listAdd(log, 'A after go'), b.wait(0.1), b.broadcast('GO'), b.listAdd(log, 'A after GO'), b.broadcast('nobody'), b.listAdd(log, 'A end')]);
        a.script(b.whenReceive('go'), [b.listAdd(log, 'A recv'), b.repeat(4, [b.change(n, 1)]), b.listAdd(log, b.join('A recv done ', n))]);
        c.script(b.whenReceive('Go'), [b.listAdd(log, 'B recv'), b.wait(0.2), b.listAdd(log, 'B recv done')]);
        p.stage.script(b.whenReceive('go'), [b.listAdd(log, 'Stage recv')]);
        // A thread restarting itself keeps running until its next yield.
        const k = p.stage.v('k', 0);
        c.script(b.whenFlag(), [b.broadcast('self')]);
        c.script(b.whenReceive('self'), [b.change(k, 1), b.if(b.lt(k, 4), [b.broadcast('self'), b.listAdd(log, b.join('self after ', k))]), b.listAdd(log, b.join('self end ', k))]);
        // Dynamic broadcast names.
        const name = p.stage.v('name', 'G');
        c.script(b.whenFlag(), [b.wait(0.3), b.broadcast(b.join(name, 'o')), b.broadcast(b.join('x', 'y')), b.listAdd(log, 'dyn sent')]);
    });

    add('broadcast_wait', p => {
        const log = p.stage.l('log', []); const n = p.stage.v('n', 0);
        const a = p.sprite('A'); const c = p.sprite('B');
        a.script(b.whenFlag(), [b.broadcastWait('work'), b.listAdd(log, b.join('work done ', n)), b.broadcastWait('none'), b.listAdd(log, 'none done'),
            b.broadcastWait('quick'), b.listAdd(log, 'quick done'), b.broadcastWait('sleep'), b.listAdd(log, b.join('sleep done ', n)),
            b.broadcastWait('nest1'), b.listAdd(log, 'nest done'), b.broadcastWait(b.join('wo', 'rk')), b.listAdd(log, b.join('dyn done ', n))]);
        a.script(b.whenFlag(), [b.forever([b.change(n, 1)])]);
        c.script(b.whenReceive('work'), [b.repeat(3, [b.listAdd(log, b.join('w', n))])]);
        p.stage.script(b.whenReceive('work'), [b.listAdd(log, 'stage work')]);
        c.script(b.whenReceive('quick'), [b.listAdd(log, 'q')]);
        c.script(b.whenReceive('sleep'), [b.wait(0.1), b.listAdd(log, 'slept')]);
        c.script(b.whenReceive('nest1'), [b.broadcastWait('nest2'), b.listAdd(log, 'n1')]);
        c.script(b.whenReceive('nest2'), [b.wait(0), b.listAdd(log, 'n2')]);
        // A waiter whose awaited thread gets restarted stops waiting for it.
        a.script(b.whenFlag(), [b.broadcastWait('long'), b.listAdd(log, b.join('long done ', n))]);
        c.script(b.whenReceive('long'), [b.wait(0.3), b.listAdd(log, 'long end')]);
        c.script(b.whenFlag(), [b.wait(0.1), b.broadcast('long')]);
    }, {frames: 40});

    add('stop', p => {
        const log = p.stage.l('log', []); const n = p.stage.v('n', 0);
        const a = p.sprite('A'); const c = p.sprite('B');
        a.script(b.whenFlag(), [b.forever([b.change(n, 1), b.if(b.gt(n, 3), [b.stop('this script')]), b.listAdd(log, b.join('a', n))])]);
        a.script(b.whenFlag(), [b.repeat(3, [b.repeat(3, [b.listAdd(log, 'in'), b.stop('this script'), b.listAdd(log, 'never')])]), b.listAdd(log, 'never')]);
        a.script(b.whenFlag(), [b.forever([b.listAdd(log, 'A other')])]);
        a.script(b.whenFlag(), [b.wait(0.1), b.stop('other scripts in sprite'), b.listAdd(log, 'A stopped others'), b.wait(0.1), b.listAdd(log, 'A still here')]);
        c.script(b.whenFlag(), [b.forever([b.listAdd(log, 'B1')])]);
        c.script(b.whenFlag(), [b.wait(0.3), b.listAdd(log, 'B stop all'), b.stop('all'), b.listAdd(log, 'never')]);
        c.script(b.whenFlag(), [b.forever([b.listAdd(log, 'B3')])]);
        p.stage.script(b.whenFlag(), [b.forever([b.listAdd(log, 'S')])]);
    }, {frames: 20});

    add('procs', p => {
        const log = p.stage.l('log', []); const n = p.stage.v('n', 0); const r = p.stage.v('r', 0);
        const a = p.sprite('A');
        a.define('say %s %s', ['x', 'y'], {}, [b.listAdd(log, b.join(b.arg('x'), b.join('|', b.arg('y'))))]);
        a.define('defaults %s %b', ['s', 'flag'], {}, [b.listAdd(log, b.join(b.arg('s'), b.join('|', b.argBool('flag')))), b.listAdd(log, b.arg('nope')), b.listAdd(log, b.not(b.argBool('flag')))]);
        a.define('dup %s %s', ['v', 'v'], {}, [b.listAdd(log, b.arg('v'))]);
        a.define('ret %s', ['k'], {}, [b.if(b.gt(b.arg('k'), 1), [b.listAdd(log, 'early'), b.stop('this script')]), b.listAdd(log, 'late')]);
        a.define('loop %s', ['k'], {}, [b.repeat(b.arg('k'), [b.change(r, 1)])]);
        a.define('fast %s', ['k'], {warp: true}, [b.repeat(b.arg('k'), [b.change(r, 10)]), b.call(a, 'loop %s', [2])]);
        a.define('rec %s', ['k'], {}, [b.if(b.gt(b.arg('k'), 0), [b.listAdd(log, b.join('rec', b.join(b.arg('k'), b.join('@', n)))), b.call(a, 'rec %s', [b.sub(b.arg('k'), 1)])])]);
        a.define('wrec %s', ['k'], {warp: true}, [b.if(b.gt(b.arg('k'), 0), [b.change(r, 100), b.call(a, 'wrec %s', [b.sub(b.arg('k'), 1)])])]);
        a.define('ping %s', ['k'], {}, [b.if(b.gt(b.arg('k'), 0), [b.listAdd(log, b.join('ping', n)), b.call(a, 'pong %s', [b.sub(b.arg('k'), 1)])])]);
        a.define('pong %s', ['k'], {}, [b.if(b.gt(b.arg('k'), 0), [b.listAdd(log, b.join('pong', n)), b.call(a, 'ping %s', [b.sub(b.arg('k'), 1)])])]);
        // Recursive call nested 7 control blocks deep: scratch-vm no longer sees it as recursive.
        const deep = k => { let body = [b.call(a, 'deep %s', [b.sub(b.arg('k'), 1)])]; for (let i = 0; i < k; i++) body = [b.if(b.eq(1, 1), body)]; return body; };
        a.define('deep %s', ['k'], {}, [b.if(b.gt(b.arg('k'), 0), [b.listAdd(log, b.join('deep', n)), ...deep(6)])]);
        a.define('deep5 %s', ['k'], {}, [b.if(b.gt(b.arg('k'), 0), [b.listAdd(log, b.join('d5-', n)), ...deep(3).map(x => x)])]);
        a.script(b.whenFlag(), [b.forever([b.change(n, 1)])]);
        a.script(b.whenFlag(), [
            b.call(a, 'say %s %s', ['a', 1]), b.call(a, 'say %s %s', [b.add(1, 2), b.eq(1, 1)]), b.call(a, 'defaults %s %b', []), b.call(a, 'dup %s %s', ['first', 'second']),
            b.call(a, 'ret %s', [1]), b.call(a, 'ret %s', [2]), b.listAdd(log, b.join('n=', n)), b.call(a, 'loop %s', [3]), b.listAdd(log, b.join('loop n=', n)),
            b.call(a, 'fast %s', [5]), b.listAdd(log, b.join('fast n=', b.join(n, b.join(' r=', r)))), b.call(a, 'rec %s', [3]), b.listAdd(log, b.join('rec n=', n)),
            b.call(a, 'wrec %s', [4]), b.listAdd(log, b.join('wrec n=', b.join(n, b.join(' r=', r)))), b.call(a, 'ping %s', [4]), b.listAdd(log, b.join('pp n=', n)),
            b.call(a, 'deep %s', [3]), b.listAdd(log, b.join('deep n=', n)), b.call(a, 'missing %s', [1]), b.listAdd(log, b.arg('outside'))
        ]);
    }, {frames: 40});

    add('clones', p => {
        const log = p.stage.l('log', []); const n = p.stage.v('n', 0);
        const a = p.sprite('A'); const c = p.sprite('B');
        const id = a.v('id', 0); const mine = a.l('mine', ['x']);
        a.script(b.whenFlag(), [b.repeat(3, [b.change(id, 1), b.listAdd(mine, id), b.clone()]), b.listAdd(log, 'made 3'), b.wait(0.2), b.broadcast('report'), b.wait(0.1), b.broadcast('die')]);
        a.script(b.whenClone(), [b.listAdd(log, b.join('born ', id)), b.change(id, 10), b.forever([b.change(n, 1)])]);
        a.script(b.whenReceive('report'), [b.listAdd(log, b.join('report ', b.join(id, b.join(' ', mine))))]);
        a.script(b.whenReceive('die'), [b.listAdd(log, b.join('die ', id)), b.deleteClone(), b.listAdd(log, b.join('survived ', id))]);
        c.script(b.whenFlag(), [b.wait(0.1), b.clone('A'), b.clone('Nobody'), b.clone('B'), b.listAdd(log, 'B cloned A and B')]);
        c.script(b.whenClone(), [b.listAdd(log, 'B clone born'), b.clone(), b.wait(0.05), b.deleteClone()]);
        p.stage.script(b.whenFlag(), [b.clone(), b.listAdd(log, 'stage cannot clone')]);
    }, {frames: 30});

    add('clone_limit', p => {
        const n = p.stage.v('n', 0); const made = p.stage.v('made', 0);
        const a = p.sprite('A');
        a.script(b.whenFlag(), [b.repeat(310, [b.clone(), b.change(made, 1)]), b.wait(0.1), b.stop('all')]);
        a.script(b.whenClone(), [b.change(n, 1), b.wait(0.5)]);
    }, {frames: 12, passes: [1]});

    add('layers', p => {
        const log = p.stage.l('log', []);
        const a = p.sprite('A'); const c = p.sprite('B'); const d = p.sprite('C');
        for (const [t, nm] of [[a, 'A'], [c, 'B'], [d, 'C']]) t.script(b.whenReceive('tick'), [b.listAdd(log, nm)]);
        a.script(b.whenFlag(), [b.broadcastWait('tick'), b.front(), b.listAdd(log, '|A front'), b.broadcastWait('tick'), b.back(), b.listAdd(log, '|A back'), b.broadcastWait('tick'),
            b.layers('forward', 1), b.listAdd(log, '|A fwd1'), b.broadcastWait('tick'), b.layers('backward', 5), b.listAdd(log, '|A back5'), b.broadcastWait('tick'),
            b.layers('forward', 1.7), b.listAdd(log, '|A fwd1.7'), b.broadcastWait('tick'), b.clone(), b.listAdd(log, '|A clone'), b.broadcastWait('tick')]);
    }, {frames: 40});

    add('say_promise', p => {
        const log = p.stage.l('log', []); const n = p.stage.v('n', 0);
        const a = p.sprite('A'); const c = p.sprite('B');
        a.script(b.whenFlag(), [b.forever([b.change(n, 1)])]);
        a.script(b.whenFlag(), [b.say('hello'), b.wait(0.1), b.say(0.1234), b.wait(0.1), b.say(b.div(1, 3)), b.wait(0.1), b.say(b.add(2, 3)), b.wait(0.1), b.say(''), b.think('hmm'), b.wait(0.1), b.say(b.div(1, 0)), b.say('0.1234')]);
        // A promise block in tail position of a loop skips that iteration's loop yield.
        c.script(b.whenFlag(), [b.repeat(3, [b.listAdd(log, b.join('s', n)), b.sayFor('x', 0.1)]), b.listAdd(log, b.join('end', n)),
            b.repeat(2, [b.sayFor('y', 0), b.listAdd(log, b.join('t', n))]), b.repeat(2, [b.if(b.eq(1, 1), [b.sayFor('z', 0.05)])]), b.listAdd(log, b.join('u', n)), b.sayFor(1.005, 'abc'), b.listAdd(log, b.join('v', n))]);
    }, {frames: 45});

    add('ask', p => {
        const log = p.stage.l('log', []);
        const a = p.sprite('A');
        a.script(b.whenFlag(), [b.listAdd(log, b.answer()), b.ask('name?'), b.listAdd(log, b.answer()), b.hide(), b.ask(b.add(1, 2)), b.listAdd(log, b.join(b.answer(), '!')), b.listAdd(log, b.add(b.answer(), 1))]);
        p.stage.script(b.whenFlag(), [b.ask('stage?'), b.listAdd(log, b.join('stage:', b.answer()))]);
    }, {frames: 12, answers: ['Ada', 's', '41'], passes: [1]});

    add('backdrops', p => {
        const log = p.stage.l('log', []);
        p.stage.costume('night').costume('3').costume('Day');
        const a = p.sprite('A').costume('b').costume('2').costume('next costume').costume('c');
        p.stage.script(b.whenBackdrop('night'), [b.listAdd(log, 'night!'), b.wait(0.1), b.listAdd(log, 'night done')]);
        a.script(b.whenBackdrop('NIGHT'), [b.listAdd(log, 'A night')]);
        a.script(b.whenBackdrop('day'), [b.listAdd(log, 'A day')]);
        a.script(b.whenFlag(), [b.listAdd(log, b.backdropNum()), b.switchBackdrop('night'), b.listAdd(log, b.backdropNum()), b.switchBackdropWait('night'), b.listAdd(log, 'waited'),
            b.switchBackdrop('next backdrop'), b.listAdd(log, b.backdropNum()), b.switchBackdrop(3), b.listAdd(log, b.backdropNum()), b.switchBackdrop('3'), b.listAdd(log, b.backdropNum()),
            b.switchBackdrop(b.add(1, 1)), b.listAdd(log, b.backdropNum()), b.switchBackdrop('previous backdrop'), b.listAdd(log, b.backdropNum()), b.switchBackdrop('day'), b.listAdd(log, b.backdropNum()),
            b.switchBackdrop('Day'), b.listAdd(log, b.backdropNum()), b.switchBackdrop(0), b.listAdd(log, b.backdropNum()), b.switchBackdrop(-1), b.listAdd(log, b.backdropNum()),
            b.switchBackdrop('random backdrop'), b.listAdd(log, b.backdropNum()), b.switchBackdrop('  '), b.switchBackdrop('nope'), b.switchBackdrop(b.eq(1, 1)), b.listAdd(log, b.backdropNum()),
            b.switchCostume('c'), b.listAdd(log, b.costumeNum()), b.switchCostume('2'), b.listAdd(log, b.costumeNum()), b.switchCostume(2), b.listAdd(log, b.costumeNum()),
            b.switchCostume('next costume'), b.listAdd(log, b.costumeNum()), b.switchCostume('previous costume'), b.listAdd(log, b.costumeNum()), b.nextCostume(), b.listAdd(log, b.costumeName()),
            b.switchCostume(b.add(2, 2.5)), b.listAdd(log, b.costumeNum()), b.switchCostume(b.div(1, 0)), b.listAdd(log, b.costumeNum()), b.switchCostume('1e1'), b.listAdd(log, b.costumeNum()),
            b.switchCostume(-3), b.listAdd(log, b.costumeNum()), b.switchCostume(''), b.listAdd(log, b.costumeNum()), b.switchCostume('4'), b.listAdd(log, b.costumeNum())]);
    }, {frames: 15});

    add('motion', p => {
        const log = p.stage.l('log', []);
        const a = p.sprite('A'); const c = p.sprite('B').set({x: 30, y: -40});
        a.script(b.whenFlag(), [b.gotoXY(10, 20), b.changeX(5), b.changeY(-5), b.listAdd(log, b.join(b.xpos(), b.join(',', b.ypos()))), b.point(45), b.move(10), b.listAdd(log, b.xpos()), b.listAdd(log, b.ypos()),
            b.turnRight(400), b.listAdd(log, b.dir()), b.turnLeft(1000), b.listAdd(log, b.dir()), b.point(-180), b.listAdd(log, b.dir()), b.point(b.div(1, 0)), b.listAdd(log, b.dir()), b.point(180.5), b.listAdd(log, b.dir()),
            b.setX('abc'), b.setY(b.div(0, 0)), b.listAdd(log, b.join(b.xpos(), b.join(',', b.ypos()))), b.gotoXY(1000, -1000), b.listAdd(log, b.join(b.xpos(), b.join(',', b.ypos()))),
            b.goTo('B'), b.listAdd(log, b.join(b.xpos(), b.join(',', b.ypos()))), b.goTo('_random_'), b.listAdd(log, b.join(b.xpos(), b.join(',', b.ypos()))), b.gotoXY(0, 0), b.pointTowards('B'), b.listAdd(log, b.dir()),
            b.pointTowards('_random_'), b.listAdd(log, b.dir()), b.listAdd(log, b.distanceTo('B')), b.listAdd(log, b.of('x position', 'B')), b.listAdd(log, b.of('backdrop #', '_stage_')), b.listAdd(log, b.of('nope', 'B')),
            b.gotoXY(0.1, 0.2), b.changeX(0.2), b.listAdd(log, b.xpos()), b.setX(2.0000000001), b.listAdd(log, b.xpos()), b.glide(0.2, 100, 50), b.listAdd(log, 'glided'), b.glide(0, 5, 5), b.glide(-1, 6, 6)]);
        p.stage.script(b.whenFlag(), [b.gotoXY(5, 5), b.point(10), b.listAdd(log, b.join('stage ', b.join(b.xpos(), b.dir())))]);
    }, {frames: 15, passes: [1]});

    add('sensing_of', p => {
        const log = p.stage.l('log', []); const g = p.stage.v('g', 'global');
        const a = p.sprite('A'); const c = p.sprite('B');
        const hp = c.v('hp', 100); c.v('g', 'local g');
        const who = p.stage.v('who', 'B');
        c.script(b.whenFlag(), [b.set(hp, '50'), b.clone(), b.wait(0.1), b.change(hp, 1)]);
        c.script(b.whenClone(), [b.set(hp, 'clone')]);
        a.script(b.whenFlag(), [b.wait(0.05), b.listAdd(log, b.of('hp', 'B')), b.listAdd(log, b.of('g', 'B')), b.listAdd(log, b.of('g', '_stage_')), b.listAdd(log, b.of('hp', '_stage_')),
            b.listAdd(log, b.of('hp', who)), b.listAdd(log, b.of('hp', b.join('', 'A'))), b.listAdd(log, b.of('direction', who)), b.wait(0.2), b.listAdd(log, b.of('hp', who)), b.listAdd(log, g)]);
    }, {frames: 15, passes: [1]});

    add('edge_hats', p => {
        const log = p.stage.l('log', []); const n = p.stage.v('n', 0);
        const a = p.sprite('A');
        a.script(b.whenGreater('TIMER', 0.2), [b.listAdd(log, b.join('timer>0.2 n=', n)), b.wait(0.1), b.listAdd(log, 'hat done')]);
        a.script(b.whenGreater('TIMER', b.div(n, 100)), [b.listAdd(log, b.join('dyn n=', n))]);
        a.script(b.whenFlag(), [b.forever([b.change(n, 1)])]);
        a.script(b.whenFlag(), [b.wait(0.5), b.resetTimer(), b.listAdd(log, 'reset'), b.wait(0.4), b.resetTimer()]);
    }, {frames: 40, passes: [1]});

    add('rng_order', p => {
        const log = p.stage.l('log', []); const n = p.stage.v('n', 0);
        const a = p.sprite('A'); const c = p.sprite('B');
        a.define('r %s', ['k'], {}, [b.if(b.gt(b.arg('k'), 5), [b.stop('this script')]), b.listAdd(log, b.arg('k'))]);
        a.script(b.whenFlag(), [b.repeat(b.random(2, 4), [b.listAdd(log, b.random(1, 100))]), b.wait(b.random(0.01, 0.2)), b.listAdd(log, b.random(1, 100)),
            b.call(a, 'r %s', [b.random(1, 10)]), b.call(a, 'r %s', [b.random(1, 10)]), b.call(a, 'r %s', [b.random(1, 10)]), b.listAdd(log, b.random(1, 100)),
            b.waitUntil(b.gt(b.random(1, 10), 7)), b.listAdd(log, b.random(1, 100)), b.until(b.gt(b.random(1, 10), 6), [b.change(n, 1)]), b.listAdd(log, b.random(1, 100)),
            b.glide(b.random(0.05, 0.1), b.random(1, 100), 0), b.listAdd(log, b.random(1, 100)), b.broadcastWait(b.join('m', b.random(1, 2))), b.listAdd(log, b.random(1, 100))]);
        c.script(b.whenFlag(), [b.forever([b.listAdd(log, b.join('B', b.random(1, 9)))])]);
        c.script(b.whenReceive('m1'), [b.wait(0.1)]);
        c.script(b.whenReceive('m2'), [b.wait(0.05)]);
    }, {frames: 40, seed: 7});

    add('undeclared', p => {
        const log = p.stage.l('log', []);
        const a = p.sprite('A');
        p.stage.v('shared', 'S');
        const ghost = {kind: 'var', name: 'ghost', id: 'no_such_id'};
        const byName = {kind: 'var', name: 'shared', id: 'wrong_id'};
        const glist = {kind: 'list', name: 'glist', id: 'no_such_list'};
        a.script(b.whenFlag(), [b.listAdd(log, ghost), b.change(ghost, 2), b.listAdd(log, ghost), b.listAdd(log, byName), b.set(byName, 'T'), b.listAdd(glist, 1), b.listAdd(log, b.listLen(glist)), b.clone()]);
        a.script(b.whenClone(), [b.listAdd(log, b.join('clone sees ', ghost)), b.listAdd(log, glist)]);
    }, {frames: 4, passes: [1]});

    return cases;
};
