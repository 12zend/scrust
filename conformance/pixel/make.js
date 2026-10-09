// Pixel comparison scene (Phase 3/4 Done): sprites with transforms and colour
// effects, pen lines, dots and a stamp. `node make.js` writes
// ../out/pixeltest/{project.sb3,index.html}; the page renders the scene with
// scratch-render (WebGL) and compares it with scrust.png from the same folder.
const fs = require('fs');
const path = require('path');
const {Project, b} = require('../harness/sb3');

const p = new Project();
p.extensions = ['pen'];
p.stage.costume('bg', '<svg xmlns="http://www.w3.org/2000/svg" width="480" height="360" viewBox="0 0 480 360"><rect width="480" height="360" fill="#dff3ff"/><rect y="280" width="480" height="80" fill="#7cc36a"/><circle cx="400" cy="70" r="40" fill="#ffe36b"/></svg>', [240, 180]);
const shape = (name, svg, c, props) => { const s = p.sprite(name); s.costume(name, svg, c); s.set(props); return s; };
const BOX = '<svg xmlns="http://www.w3.org/2000/svg" width="80" height="50" viewBox="0 0 80 50"><rect width="80" height="50" rx="8" fill="#4c97ff"/><rect x="10" y="10" width="25" height="30" fill="#ffffff"/><circle cx="58" cy="25" r="12" fill="#ff6680"/></svg>';
const TRI = '<svg xmlns="http://www.w3.org/2000/svg" width="60" height="60" viewBox="0 0 60 60"><polygon points="30,2 58,58 2,58" fill="#9966ff"/></svg>';
const a = shape('A', BOX, [40, 25], {x: -150, y: 110});
const c = shape('B', BOX, [40, 25], {x: -20, y: 110, direction: 60});
const d = shape('C', BOX, [40, 25], {x: 110, y: 100, size: 160});
const e = shape('D', BOX, [40, 25], {x: -150, y: 20, direction: -90, rotationStyle: 'left-right'});
const f = shape('E', TRI, [30, 30], {x: -40, y: 20});
const g = shape('F', TRI, [30, 30], {x: 50, y: 20, size: 50, direction: 135});
const h = shape('G', BOX, [40, 25], {x: 150, y: 10});
const pen = shape('Pen', '<svg xmlns="http://www.w3.org/2000/svg" width="12" height="12" viewBox="0 0 12 12"><circle cx="6" cy="6" r="6" fill="#000000"/></svg>', [6, 6], {x: -200, y: -60, visible: false});

f.script(b.whenFlag(), [b.setEffect('GHOST', 50)]);
g.script(b.whenFlag(), [b.setEffect('BRIGHTNESS', 40)]);
h.script(b.whenFlag(), [b.setEffect('COLOR', 60)]);
pen.script(b.whenFlag(), [b.penClear(),
    b.penColor('#e6194b'), b.penSize(1), b.gotoXY(-220, -40), b.penDown(), b.gotoXY(-100, -40), b.penUp(),
    b.penColor('#3cb44b'), b.penSize(4), b.gotoXY(-220, -60), b.penDown(), b.gotoXY(-100, -90), b.gotoXY(-220, -120), b.penUp(),
    b.penColor('#4363d8'), b.penSize(12), b.gotoXY(-60, -50), b.penDown(), b.gotoXY(40, -110), b.penUp(),
    b.penColor('#f58231'), b.penSize(30), b.gotoXY(100, -70), b.penDown(), b.penUp(),
    b.penColor('#911eb4'), b.penSize(8), b.penParam('transparency', 50), b.gotoXY(60, -130), b.penDown(), b.gotoXY(200, -40), b.penUp(),
    b.gotoXY(180, -110), b.show(), b.setSize(300), b.penStamp(), b.hide()]);

const HTML = `<!doctype html>
<meta charset="utf-8"><title>scrust pixel reference</title>
<canvas id="c" width="480" height="360"></canvas><canvas id="d" width="480" height="360"></canvas><pre id="log">running…</pre>
<script src="https://cdn.jsdelivr.net/npm/scratch-vm@5.0.300/dist/web/scratch-vm.min.js"></script>
<script src="https://cdn.jsdelivr.net/npm/scratch-render@2.2.84/dist/web/scratch-render.min.js"></script>
<script src="https://cdn.jsdelivr.net/npm/scratch-storage@latest/dist/web/scratch-storage.min.js"></script>
<script>
(async () => {
  const VM = window.VirtualMachine;
  const S = window.ScratchStorage;
  const Storage = (S && (S.ScratchStorage || S.default)) || S;
  const canvas = document.getElementById('c');
  const vm = new VM();
  const renderer = new (window.ScratchRender || window.RenderWebGL)(canvas);
  vm.attachRenderer(renderer);
  vm.attachStorage(new Storage());
  await vm.loadProject(await (await fetch('project.sb3')).arrayBuffer());
  await new Promise(r => setTimeout(r, 1500));
  const rt = vm.runtime;
  rt.currentStepTime = 1000 / 30;
  vm.greenFlag();
  for (let i = 0; i < 3; i++) { try { rt._step(); } catch (e) { console.error(e); } await new Promise(r => setTimeout(r, 50)); }
  // resize() multiplies by devicePixelRatio; we want a 480x360 drawing buffer.
  renderer.resize(480 / (window.devicePixelRatio || 1), 360 / (window.devicePixelRatio || 1));
  renderer.draw();
  const gl = renderer.gl;
  const ref = new Uint8Array(480 * 360 * 4);
  gl.readPixels(0, 0, 480, 360, gl.RGBA, gl.UNSIGNED_BYTE, ref);
  // Save the reference frame (serve.js stores it as conformance/pixel/reference.png).
  {
    const flip = document.createElement('canvas'); flip.width = 480; flip.height = 360;
    const fctx = flip.getContext('2d');
    const id = fctx.createImageData(480, 360);
    for (let y = 0; y < 360; y++) id.data.set(ref.subarray((359 - y) * 1920, (360 - y) * 1920), y * 1920);
    for (let i = 3; i < id.data.length; i += 4) id.data[i] = 255;
    fctx.putImageData(id, 0, 0);
    const blob = await new Promise(r => flip.toBlob(r, 'image/png'));
    try { await fetch('/reference.png', {method: 'POST', body: blob}); } catch (e) { /* plain static server */ }
  }
  const img = await createImageBitmap(await (await fetch('scrust.png?' + Date.now())).blob());
  const ctx = document.getElementById('d').getContext('2d', {willReadFrequently: true});
  ctx.drawImage(img, 0, 0);
  const got = ctx.getImageData(0, 0, 480, 360).data;
  let sum = 0; let over32 = 0; let over96 = 0; let max = 0;
  const thirds = [0, 0, 0];
  for (let y = 0; y < 360; y++) {
    for (let x = 0; x < 480; x++) {
      const gi = (y * 480 + x) * 4;
      const ri = ((359 - y) * 480 + x) * 4; // readPixels is bottom-up
      let dmax = 0;
      for (let k = 0; k < 3; k++) { const dd = Math.abs(got[gi + k] - ref[ri + k]); sum += dd; if (dd > dmax) dmax = dd; }
      if (dmax > 32) { over32++; thirds[Math.floor(y / 120)]++; }
      if (dmax > 96) over96++;
      if (dmax > max) max = dmax;
    }
  }
  const n = 480 * 360;
  window.__result = {meanAbsDiff: +(sum / (n * 3)).toFixed(3), pctOver32: +(100 * over32 / n).toFixed(3), pctOver96: +(100 * over96 / n).toFixed(3), max, over32ByBand: thirds};
  document.getElementById('log').textContent = JSON.stringify(window.__result);
})().catch(e => { window.__result = {error: String(e && e.stack || e)}; document.getElementById('log').textContent = String(e && e.stack || e); });
</script>`;

p.sb3().then(buf => {
    const dir = path.join(__dirname, '..', 'out', 'pixeltest');
    fs.mkdirSync(dir, {recursive: true});
    fs.writeFileSync(path.join(dir, 'project.sb3'), buf);
    fs.writeFileSync(path.join(dir, 'index.html'), HTML);
    console.log('wrote', dir);
});
