// Builds conformance/out/demo.sb3: sprites with SVG costumes, pen, clones,
// say bubbles, a variable monitor and graphic effects.
const fs = require('fs');
const path = require('path');
const {Project, b, toneWav} = require('../harness/sb3');

const p = new Project();
p.stage.costume('sky', '<svg xmlns="http://www.w3.org/2000/svg" width="480" height="360" viewBox="0 0 480 360"><defs><linearGradient id="g" x1="0" y1="0" x2="0" y2="1"><stop offset="0" stop-color="#8ecbff"/><stop offset="1" stop-color="#e8f6ff"/></linearGradient></defs><rect width="480" height="360" fill="url(#g)"/><rect y="300" width="480" height="60" fill="#6bbf59"/></svg>', [240, 180]);
const ball = p.sprite('Ball');
ball.costume('ball', '<svg xmlns="http://www.w3.org/2000/svg" width="48" height="48" viewBox="0 0 48 48"><circle cx="24" cy="24" r="22" fill="#ff6680" stroke="#b3475a" stroke-width="3"/><circle cx="16" cy="16" r="6" fill="#ffffff" fill-opacity="0.7"/></svg>', [24, 24]);
const arrow = p.sprite('Arrow');
arrow.costume('arrow', '<svg xmlns="http://www.w3.org/2000/svg" width="60" height="30" viewBox="0 0 60 30"><path d="M2 10 H38 V2 L58 15 L38 28 V20 H2 Z" fill="#4c97ff" stroke="#3373cc" stroke-width="2"/></svg>', [30, 15]);
arrow.costume('arrow2', '<svg xmlns="http://www.w3.org/2000/svg" width="60" height="30" viewBox="0 0 60 30"><path d="M2 10 H38 V2 L58 15 L38 28 V20 H2 Z" fill="#ffab19" stroke="#cf8b17" stroke-width="2"/></svg>', [30, 15]);
const star = p.sprite('Star');
star.costume('star', '<svg xmlns="http://www.w3.org/2000/svg" width="40" height="40" viewBox="0 0 40 40"><polygon points="20,2 25,15 38,15 27,23 31,37 20,28 9,37 13,23 2,15 15,15" fill="#ffd500" stroke="#cc9900" stroke-width="2"/></svg>', [20, 20]);

const score = p.stage.v('score', 0);
const hits = p.stage.v('hits', 0);
p.monitors.push({id: score.id, mode: 'default', opcode: 'data_variable', params: {VARIABLE: 'score'}, spriteName: null, value: 0, width: 0, height: 0, x: 5, y: 5, visible: true, sliderMin: 0, sliderMax: 100, isDiscrete: true});
p.monitors.push({id: hits.id, mode: 'large', opcode: 'data_variable', params: {VARIABLE: 'hits'}, spriteName: null, value: 0, width: 0, height: 0, x: 5, y: 32, visible: true, sliderMin: 0, sliderMax: 100, isDiscrete: true});

ball.sound('pop', toneWav(660, 0.15));
ball.set({x: -100, y: 60, direction: 60, size: 100});
ball.script(b.whenFlag(), [b.penClear(), b.penColor('#3355ff'), b.penSize(3), b.penDown(),
    b.forever([b.move(7), b.bounce(), b.change(score, 1), b.penChangeParam('color', 2),
        b.if(b.touching('Arrow'), [b.change(hits, 1), b.say(b.join('hit ', hits)), b.playSound('pop')])])]);
arrow.set({x: 60, y: -40});
arrow.script(b.whenFlag(), [b.forever([b.turnRight(6), b.changeEffect('COLOR', 3), b.if(b.eq(b.mod(score, 20), 0), [b.nextCostume()])])]);
star.set({x: -180, y: 120, size: 80});
star.script(b.whenFlag(), [b.repeat(6, [b.clone(), b.wait(0.2)]), b.think('Hmm...')]);
star.script(b.whenClone(), [b.gotoXY(b.random(-200, 200), 170), b.setEffect('GHOST', 30), b.setSize(b.random(40, 120)),
    b.forever([b.changeY(-3), b.turnRight(5), b.if(b.lt(b.ypos(), -130), [b.gotoXY(b.random(-200, 200), 170)])])]);

p.sb3().then(buf => {
    const out = path.join(__dirname, '..', 'out');
    fs.mkdirSync(out, {recursive: true});
    fs.writeFileSync(path.join(out, 'demo.sb3'), buf);
    console.log('wrote', path.join(out, 'demo.sb3'), buf.length, 'bytes');
});
