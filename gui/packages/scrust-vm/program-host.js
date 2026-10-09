/**
 * Client for a compiled project running as a native process (started by the
 * Electron main process with `--embed`, see crates/scrust-player/src/embed.rs).
 * Input goes out as text lines; frames, the number of running threads and
 * state snapshots come back as messages.
 */

const encoder = new TextEncoder();

const api = () => (typeof window !== 'undefined' ? window.scrustCompiler : null);

const hex = s => Array.from(encoder.encode(s), b => b.toString(16).padStart(2, '0')).join('');

/** The one running program; messages are routed to it. */
let current = null;
let listening = false;

class ProgramHost {
    /** @param {{onFrame: Function, onState: Function, onExit: Function}} handlers */
    constructor (handlers) {
        this.bridge = api();
        this.handlers = handlers;
        this.threads = 0;
        this.stateRequests = [];
        this.disposed = false;
    }

    /**
     * Start the last compiled program; it receives the green flag at once.
     * @param {{scale: number, sharedFrames: boolean}} options
     * @returns {Promise<{framesPath: ?string}>} the shared frame file, when asked for
     */
    async start (options) {
        if (!listening) {
            listening = true;
            this.bridge.onMessage(message => {
                if (current) current._handle(message);
            });
        }
        current = this;
        return (await this.bridge.start(options)) || {framesPath: null};
    }

    _handle (message) {
        switch (message.type) {
        case 'frame':
            this.handlers.onFrame(message.data, message.width, message.height);
            break;
        case 'threads':
            this.threads = message.count;
            break;
        case 'state': {
            const state = JSON.parse(message.json);
            const requests = this.stateRequests;
            this.stateRequests = [];
            if (requests.length) requests.forEach(resolve => resolve(state));
            else this.handlers.onState(state);
            break;
        }
        case 'exit':
            this.disposed = true;
            if (current === this) current = null;
            this.stateRequests.forEach(resolve => resolve(null));
            this.stateRequests = [];
            this.handlers.onExit(message);
            break;
        }
    }

    _send (line) {
        if (!this.disposed) this.bridge.send(line);
    }

    mouseMove (x, y) {
        this._send(`move ${x} ${y}`);
    }

    mouseButton (x, y, down) {
        this._send(`button ${x} ${y} ${down ? 1 : 0}`);
    }

    key (key, down) {
        if (key.length) this._send(`key ${down ? 1 : 0} ${hex(key)}`);
    }

    greenFlag () {
        this._send('flag');
    }

    stop () {
        this._send('stop');
    }

    threadCount () {
        return this.threads;
    }

    /** @returns {Promise<?object>} the program's state (see crates/scrust-player/src/state.rs) */
    requestState () {
        if (this.disposed) return Promise.resolve(null);
        return new Promise(resolve => {
            this.stateRequests.push(resolve);
            this._send('state');
        });
    }

    dispose () {
        if (this.disposed) return;
        this.disposed = true;
        if (current === this) current = null;
        this.bridge.kill();
    }
}

module.exports = ProgramHost;
