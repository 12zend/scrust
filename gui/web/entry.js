// Entry of the browser build of the scrust editor (gui/build-web). It loads
// the scene module and installs the browser bridges (./bridge.js) where the
// Electron preload would, then starts the same editor as the Electron app.
//
// SCRUST_COMPILER_URL (set when building) is where the compile server is;
// empty means the server that serves this page (gui/web/server.js).

const {loadScene, createCompiler} = require('./bridge');

const compileUrl = process.env.SCRUST_COMPILER_URL ?
    `${process.env.SCRUST_COMPILER_URL.replace(/\/+$/, '')}/api/compile` :
    'api/compile';

loadScene('scrust-scene.wasm')
    .then(scene => {
        window.scrustNative = {scene};
        window.scrustCompiler = createCompiler(compileUrl);
        // Evaluated only now: scratch-render looks for the scene when it loads.
        require('../src/entry.js');
    })
    .catch(e => {
        document.body.textContent = `scrust: failed to load the renderer: ${e.message || e}`;
    });
