// Bridges the GUI page to scrust:
// - scrustNative.scene: the editor's renderer (crates/scrust-node, a Node-API
//   addon over scrust-render), called synchronously;
// - scrustCompiler: compile the project and run the result as a child
//   process of the main process, which streams its frames back here.
const {contextBridge, ipcRenderer} = require('electron');
const path = require('path');

const ADDON_NAMES = {darwin: 'libscrust_node.dylib', win32: 'scrust_node.dll'};
const addonPath = process.env.SCRUST_NODE_ADDON ||
    path.join(__dirname, '..', '..', 'target', 'release', ADDON_NAMES[process.platform] || 'libscrust_node.so');

const scene = {};
try {
    const mod = {exports: {}};
    process.dlopen(mod, addonPath);
    Object.assign(scene, mod.exports);
} catch (e) {
    scene.loadError = () => `${addonPath}: ${e.message}`;
}

contextBridge.exposeInMainWorld('scrustNative', {scene});

contextBridge.exposeInMainWorld('scrustCompiler', {
    /**
     * Compile the project to a native executable.
     * @param {Uint8Array} sb3 the project
     * @param {{fps: number, turbo: boolean}} options
     * @returns {Promise<{ok: boolean, warnings?: string, log: string}>}
     */
    compile: (sb3, options) => ipcRenderer.invoke('scrust:compile', sb3, options),
    /** Start the last compiled program (it receives the green flag); resolves to {framesPath}. */
    start: options => ipcRenderer.invoke('scrust:start', options),
    /** One input line for the program (see crates/scrust-player/src/embed.rs). */
    send: line => ipcRenderer.send('scrust:input', line),
    kill: () => ipcRenderer.send('scrust:kill'),
    /** Messages from the program: {type: 'frame'|'threads'|'state'|'exit', ...}. */
    onMessage: callback => {
        ipcRenderer.on('scrust:program', (event, message) => {
            try {
                callback(message);
            } finally {
                // The main process sends the next frame once this one is drawn.
                if (message.type === 'frame') ipcRenderer.send('scrust:frame-ack');
            }
        });
    }
});
