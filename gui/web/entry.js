// Entry of the browser build of the scrust editor (gui/build-web). It loads
// the scene module and installs the browser bridges (./bridge.js) where the
// Electron preload would, then starts the same editor as the Electron app.
// Projects run in the page with scrust's interpreter (scrust-interp.wasm):
// the build is plain static files and needs no server of its own.

const {loadScene, createInterpreter} = require('./bridge');

loadScene('scrust-scene.wasm')
    .then(scene => {
        window.scrustNative = {scene};
        window.scrustCompiler = createInterpreter('scrust-interp.wasm');
        // Evaluated only now: scratch-render looks for the scene when it loads.
        require('../src/entry.js');
    })
    .catch(e => {
        document.body.textContent = `scrust: failed to load the renderer: ${e.message || e}`;
    });
