// Entry of the scrust editor bundle. The stage renderer is a native addon
// loaded by the Electron preload; without it the editor cannot draw, so say
// so instead of starting.
const ScrustRender = require('scratch-render');

ScrustRender.ready
    .then(() => {
        require('../vendor/scratch-gui/src/playground/editor.jsx');
    })
    .catch(e => {
        document.body.textContent = `scrust: failed to load the renderer: ${e.message || e}`;
    });
