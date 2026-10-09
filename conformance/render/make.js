// Renderer-dependent conformance project: fencing, size clamping, bounds,
// `touching` blocks, edge bounce. Results are appended to the stage list `out`.
//   node make.js  ->  ../out/rendertest/project.sb3 (+ index.html for the browser reference run)
const fs = require('fs');
const path = require('path');
const {Project, b} = require('../harness/sb3');

const p = new Project();
p.stage.costume('bg', '<svg xmlns="http://www.w3.org/2000/svg" width="480" height="360" viewBox="0 0 480 360"><rect width="480" height="360" fill="#ffffff"/><rect x="300" y="100" width="100" height="80" fill="#ff0000"/><rect x="40" y="250" width="60" height="60" fill="#0000ff"/></svg>', [240, 180]);
const out = p.stage.l('out', []);
const box = p.sprite('Box');
box.costume('box', '<svg xmlns="http://www.w3.org/2000/svg" width="40" height="20" viewBox="0 0 40 20"><rect width="40" height="20" fill="#00aa00"/></svg>', [20, 10]);
const dot = p.sprite('Dot');
dot.costume('dot', '<svg xmlns="http://www.w3.org/2000/svg" width="20" height="20" viewBox="0 0 20 20"><circle cx="10" cy="10" r="10" fill="#ffaa00"/></svg>', [10, 10]);
const big = p.sprite('Big');
big.costume('big', '<svg xmlns="http://www.w3.org/2000/svg" width="200" height="100" viewBox="0 0 200 100"><rect width="200" height="100" fill="#8800ff" fill-opacity="0.5"/></svg>', [100, 50]);
big.set({x: 0, y: -120, visible: false});
dot.set({x: 0, y: 0});
box.set({x: 0, y: 100});

const log = e => b.listAdd(out, e);
const pos = () => b.join(b.xpos(), b.join(',', b.ypos()));
const at = (x, y) => [b.gotoXY(x, y), log(pos())];

box.script(b.whenFlag(), [
    // --- fencing ---
    ...at(1000, 0), ...at(-1000, 0), ...at(0, 1000), ...at(0, -1000), ...at(1000, 1000), ...at(235, 175), ...at(250, 0), ...at(229.5, 0), ...at(-250.7, -190.2),
    b.point(45), ...at(1000, 0), ...at(-1000, -1000), b.point(0), ...at(1000, 0), ...at(0, 1000), b.point(90),
    b.gotoXY(0, 0), b.changeX(600), log(pos()), b.changeY(-600), log(pos()), b.gotoXY(0, 0), b.move(-700), log(pos()),
    // --- size clamping ---
    b.setSize(500), log(B_size()), b.setSize(5000), log(B_size()), b.setSize(1), log(B_size()), b.setSize(-50), log(B_size()), b.setSize(12.5), log(B_size()), b.setSize(100), log(B_size()),
    b.setSize(300), ...at(1000, 0), b.setSize(100),
    // --- touching sprite ---
    ...[[0, 0], [25, 0], [29, 0], [30, 0], [31, 0], [35, 0], [0, 19], [0, 20], [0, 21], [-29, 0], [-30, 0], [-31, 0], [24, 14], [26, 16], [28, 17]].flatMap(([x, y]) => [b.gotoXY(x, y), log(b.touching('Dot'))]),
    b.gotoXY(0, 0), b.point(45), log(b.touching('Dot')), b.gotoXY(22, 0), log(b.touching('Dot')), b.gotoXY(30, 0), log(b.touching('Dot')), b.point(90),
    b.gotoXY(0, 0), b.setSize(50), log(b.touching('Dot')), b.gotoXY(19, 0), log(b.touching('Dot')), b.gotoXY(21, 0), log(b.touching('Dot')), b.setSize(100),
    b.gotoXY(0, 0), b.setEffect('GHOST', 100), log(b.touching('Dot')), b.setEffect('GHOST', 0), b.hide(), log(b.touching('Dot')), b.show(),
    log(b.touching('Big')), log(b.touching('Nobody')),
    // --- touching edge / bounce ---
    b.gotoXY(0, 0), log(b.touching('_edge_')), b.gotoXY(219, 0), log(b.touching('_edge_')), b.gotoXY(220, 0), log(b.touching('_edge_')), b.gotoXY(221, 0), log(b.touching('_edge_')),
    b.gotoXY(0, 169), log(b.touching('_edge_')), b.gotoXY(0, 171), log(b.touching('_edge_')), b.gotoXY(0, -171), log(b.touching('_edge_')),
    b.gotoXY(240, 0), b.point(60), b.bounce(), log(b.join(b.dir(), b.join('@', pos()))), b.gotoXY(0, 200), b.point(10), b.bounce(), log(b.join(b.dir(), b.join('@', pos()))),
    b.gotoXY(0, 0), b.point(30), b.bounce(), log(b.join(b.dir(), b.join('@', pos()))), b.gotoXY(-300, -300), b.point(-135), b.bounce(), log(b.join(b.dir(), b.join('@', pos()))), b.point(90),
    // --- touching colour (backdrop: red block at x 60..160, y 0..80; blue block at x -200..-140, y -130..-70) ---
    b.gotoXY(0, 0), log(b.touchingColor('#ff0000')), b.gotoXY(100, 40), log(b.touchingColor('#ff0000')), log(b.touchingColor('#0000ff')), log(b.touchingColor('#ffffff')),
    b.gotoXY(45, 40), log(b.touchingColor('#ff0000')), b.gotoXY(39, 40), log(b.touchingColor('#ff0000')), b.gotoXY(40, 40), log(b.touchingColor('#ff0000')), b.gotoXY(41, 40), log(b.touchingColor('#ff0000')),
    b.gotoXY(-170, -100), log(b.touchingColor('#0000ff')), log(b.touchingColor('#0000f0')), log(b.touchingColor('#0808ff')), log(b.touchingColor('#1010ff')),
    b.gotoXY(0, 0), log(b.touchingColor('#ffaa00')), b.gotoXY(100, 100), log(b.touchingColor('#ffaa00')),
    log('done')
]);
function B_size () { return {opcode: 'looks_size', inputs: {}, fields: {}}; }

dot.script(b.whenFlag(), []);
big.script(b.whenFlag(), [b.setSize(1000), b.listAdd(out, b.join('big ', B_size())), b.setSize(0), b.listAdd(out, b.join('big ', B_size())), b.setSize(100), b.gotoXY(1000, 0), b.listAdd(out, b.join('big ', pos())), b.gotoXY(0, -1000), b.listAdd(out, b.join('big ', pos()))]);

const HTML = `<!doctype html>
<meta charset="utf-8"><title>scrust render reference</title>
<canvas id="c" width="480" height="360"></canvas><pre id="log">running…</pre>
<script src="https://cdn.jsdelivr.net/npm/scratch-vm@5.0.300/dist/web/scratch-vm.min.js"></script>
<script src="https://cdn.jsdelivr.net/npm/scratch-render@2.2.84/dist/web/scratch-render.min.js"></script>
<script src="https://cdn.jsdelivr.net/npm/scratch-storage@latest/dist/web/scratch-storage.min.js"></script>
<script src="https://cdn.jsdelivr.net/npm/scratch-svg-renderer@latest/dist/web/scratch-svg-renderer.js"></script>
<script>
(async () => {
  const VM = window.VirtualMachine || window.ScratchVM || (window.Scratch && window.Scratch.VirtualMachine);
  const Render = window.ScratchRender || window.RenderWebGL;
  const S = window.ScratchStorage;
  const Storage = (S && (S.ScratchStorage || S.default)) || S;
  const vm = new VM();
  vm.attachRenderer(new Render(document.getElementById('c')));
  vm.attachStorage(new Storage());
  if (window.ScratchSVGRenderer) {
    if (ScratchSVGRenderer.BitmapAdapter) vm.attachV2BitmapAdapter(new ScratchSVGRenderer.BitmapAdapter());
  }
  const buf = await (await fetch('project.sb3')).arrayBuffer();
  await vm.loadProject(buf);
  const rt = vm.runtime;
  // SVG skins finish loading asynchronously (image decode); a user would only
  // click the green flag afterwards.
  await new Promise(r => setTimeout(r, 1500));
  let now = 1577836800000;
  Date.now = () => now;
  rt.sequencer.timer = {start () { this.n = 0; }, timeElapsed () { return this.n++ >= 1 ? Infinity : 0; }};
  rt.currentStepTime = 1000 / 30;
  rt.updateCurrentMSecs();
  vm.greenFlag();
  for (let f = 0; f < 3; f++) { now += 1000 / 30; try { rt._step(); } catch (e) { console.error(e); } await new Promise(r => setTimeout(r, 30)); }
  const stage = rt.getTargetForStage();
  const out = Object.values(stage.variables).find(v => v.name === 'out').value.map(v => (typeof v === 'number' && Object.is(v, -0)) ? '-0' : String(v));
  window.__result = out;
  document.getElementById('log').textContent = JSON.stringify(out);
})().catch(e => { window.__result = {error: String(e && e.stack || e)}; document.getElementById('log').textContent = String(e && e.stack || e); });
</script>`;

p.sb3().then(buf => {
    const dir = path.join(__dirname, '..', 'out', 'rendertest');
    fs.mkdirSync(dir, {recursive: true});
    fs.writeFileSync(path.join(dir, 'project.sb3'), buf);
    fs.writeFileSync(path.join(dir, 'index.html'), HTML);
    console.log('wrote', dir);
});
