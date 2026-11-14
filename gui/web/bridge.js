/**
 * Browser counterparts of what the Electron preload (gui/electron/preload.js)
 * gives the page, with the same interfaces, so gui/packages/scrust-vm and
 * gui/packages/scrust-render run unchanged:
 *
 * - `scrustNative.scene`: the editor's renderer, scrust_render::Scene built
 *   for wasm32 (crates/scrust-scene-wasm) instead of the Node-API addon;
 * - `scrustCompiler`: there is no rustc in a browser, so the green flag
 *   hands the project to scrust's interpreter (crates/scrust-interp), one
 *   WebAssembly module with the front end, the runtime and the renderer: it
 *   loads the .sb3 and runs it in the page, driven once per animation frame,
 *   instead of a compiled native child process. Nothing leaves the browser.
 *   Input arrives as the embed protocol's text lines
 *   (crates/scrust-player/src/embed.rs) and messages go back in the shape the
 *   Electron main process sends.
 */

const encoder = new TextEncoder();
const decoder = new TextDecoder();

const fetchBytes = async url => {
    const res = await fetch(url);
    if (!res.ok) throw new Error(`${url}: HTTP ${res.status}`);
    return res.arrayBuffer();
};

// ------------------------------------------------------------------ scene

/**
 * Load the scene module and wrap it in the interface of the renderer addon
 * (crates/scrust-node).
 * @param {string} url of scrust-scene.wasm
 */
async function loadScene (url) {
    const {instance} = await WebAssembly.instantiate(await fetchBytes(url), {});
    const ex = instance.exports;
    // Views are made after each call: the module's memory may have grown.
    const put = bytes => {
        const ptr = ex.scene_input(bytes.length);
        new Uint8Array(ex.memory.buffer, ptr, bytes.length).set(bytes);
        return bytes.length;
    };
    const putIds = ids => {
        const list = ids instanceof Uint32Array ? ids : new Uint32Array(ids);
        put(new Uint8Array(list.buffer, list.byteOffset, list.byteLength));
        return list.length;
    };
    const out = n => Array.from(new Float64Array(ex.memory.buffer, ex.scene_out(), 8).subarray(0, n));
    const flag = v => (v ? 1 : 0);

    return {
        resize: (w, h) => ex.scene_resize(w, h),
        usesGpu: () => 0,
        background: (r, g, b) => ex.scene_background(r, g, b),
        svgSkin: (id, svg, has, cx, cy) => ex.scene_svg_skin(id, put(encoder.encode(svg)), flag(has), cx, cy),
        bitmapSkin: (id, w, h, rgba, resolution, has, cx, cy) => {
            put(new Uint8Array(rgba.buffer, rgba.byteOffset, w * h * 4));
            return ex.scene_bitmap_skin(id, w, h, resolution, flag(has), cx, cy);
        },
        emptySkin: () => ex.scene_empty_skin(),
        destroySkin: id => ex.scene_destroy_skin(id),
        skinInfo: id => (ex.scene_skin_info(id) ? out(4) : null),
        createDrawable: () => ex.scene_create_drawable(),
        destroyDrawable: id => ex.scene_destroy_drawable(id),
        drawableSkin: (id, skin) => ex.scene_drawable_skin(id, skin),
        drawablePosition: (id, x, y) => ex.scene_drawable_position(id, x, y),
        drawableDirection: (id, d) => ex.scene_drawable_direction(id, d),
        drawableScale: (id, x, y) => ex.scene_drawable_scale(id, x, y),
        drawableVisible: (id, v) => ex.scene_drawable_visible(id, flag(v)),
        drawableEffect: (id, name, value) => ex.scene_drawable_effect(id, put(encoder.encode(name)), value),
        bounds: (id, tight) => (ex.scene_bounds(id, flag(tight)) ? out(4) : null),
        fenced: (id, x, y) => {
            ex.scene_fenced(id, x, y);
            return out(2);
        },
        pick: (ids, x, y) => ex.scene_pick(putIds(ids), x, y),
        // A view of the module's memory, used at once by the caller.
        draw: ids => {
            const ptr = ex.scene_draw(putIds(ids));
            return new Uint8Array(ex.memory.buffer, ptr, ex.scene_rgba_len());
        },
        extract: id => {
            if (!ex.scene_extract(id)) return null;
            const [x, y, w, h] = out(4);
            return [x, y, w, h, new Uint8Array(ex.memory.buffer, ex.scene_extracted(), w * h * 4).slice()];
        }
    };
}

// ------------------------------------------------------------------ sound

/**
 * Web Audio for one running program: the imports of crates/scrust-player/src/web.rs
 * (the same glue as crates/scrust-player/web/index.html).
 */
class ProgramAudio {
    constructor () {
        this.ex = null;
        this.ctx = null;
        this.decoded = new Map();
        this.playing = new Map();
        this.closed = false;
    }

    ensure () {
        if (!this.ctx) {
            const AC = window.AudioContext || window.webkitAudioContext;
            if (AC) this.ctx = new AC();
        }
        if (this.ctx && this.ctx.state === 'suspended') this.ctx.resume();
        return this.ctx;
    }

    buffer (sprite, sound) {
        const key = `${sprite}/${sound}`;
        if (!this.decoded.has(key)) {
            const ex = this.ex;
            const ptr = ex.scrust_sound_ptr(sprite, sound);
            const len = ex.scrust_sound_len(sprite, sound);
            const bytes = new Uint8Array(ex.memory.buffer, ptr, len).slice().buffer;
            this.decoded.set(key, new Promise((resolve, reject) => this.ctx.decodeAudioData(bytes, resolve, reject)).catch(() => null));
        }
        return this.decoded.get(key);
    }

    done (id) {
        if (!this.closed) this.ex.scrust_sound_done(id);
    }

    play (id, sprite, sound, volume, rate, pan) {
        const ctx = this.ensure();
        if (!ctx) {
            queueMicrotask(() => this.done(id));
            return;
        }
        const entry = {source: null, gain: ctx.createGain(), pan: ctx.createStereoPanner ? ctx.createStereoPanner() : null, stopped: false, rate};
        entry.gain.gain.value = volume;
        if (entry.pan) {
            entry.pan.pan.value = pan;
            entry.gain.connect(entry.pan);
            entry.pan.connect(ctx.destination);
        } else {
            entry.gain.connect(ctx.destination);
        }
        this.playing.set(id, entry);
        this.buffer(sprite, sound).then(buffer => {
            if (entry.stopped || !buffer || this.closed) {
                this.playing.delete(id);
                this.done(id);
                return;
            }
            const src = ctx.createBufferSource();
            src.buffer = buffer;
            src.playbackRate.value = entry.rate;
            src.connect(entry.gain);
            src.onended = () => {
                if (this.playing.get(id) === entry) {
                    this.playing.delete(id);
                    this.done(id);
                }
            };
            entry.source = src;
            src.start();
        });
    }

    stop (id) {
        const e = this.playing.get(id);
        if (!e) return;
        e.stopped = true;
        this.playing.delete(id);
        if (e.source) {
            e.source.onended = null;
            try {
                e.source.stop();
            } catch (err) {
                // already stopped
            }
        }
    }

    set (id, volume, rate, pan) {
        const e = this.playing.get(id);
        if (!e) return;
        e.gain.gain.value = volume;
        e.rate = rate;
        if (e.source) e.source.playbackRate.value = rate;
        if (e.pan) e.pan.pan.value = pan;
    }

    close () {
        this.closed = true;
        for (const id of Array.from(this.playing.keys())) this.stop(id);
        if (this.ctx) this.ctx.close().catch(() => {});
    }
}

// ------------------------------------------------------------------ program

const unhex = hex => {
    const bytes = new Uint8Array(hex.length >> 1);
    for (let i = 0; i < bytes.length; i++) bytes[i] = parseInt(hex.substr(i * 2, 2), 16);
    return decoder.decode(bytes);
};

/** One run of a project in this page: a fresh instance of the interpreter. */
class WasmProgram {
    constructor (module, post) {
        this.module = module;
        this.post = post;
        this.ex = null;
        this.audio = new ProgramAudio();
        this.panic = '';
        this.lastThreads = null;
        this.raf = 0;
        this.started = false;
        this.stopped = false;
    }

    /**
     * Instantiate the interpreter and load the project into it.
     * @returns {Promise<{ok: boolean, message: string}>} unsupported blocks, or the error
     */
    async load (sb3, options) {
        const audio = this.audio;
        const env = {
            js_sound_play: (...a) => audio.play(...a),
            js_sound_stop: id => audio.stop(id),
            js_sound_set: (...a) => audio.set(...a),
            js_panic: (ptr, len) => {
                this.panic = decoder.decode(new Uint8Array(this.ex.memory.buffer, ptr, len));
            }
        };
        const instance = await WebAssembly.instantiate(this.module, {env});
        const ex = instance.exports;
        this.ex = ex;
        audio.ex = ex;
        try {
            const ptr = ex.scrust_input(sb3.length);
            new Uint8Array(ex.memory.buffer, ptr, sb3.length).set(sb3);
            const ok = ex.scrust_load(options.fps || 0, options.turbo ? 1 : 0);
            const message = decoder.decode(new Uint8Array(ex.memory.buffer, ex.scrust_message_ptr(), ex.scrust_message_len()));
            return {ok: !!ok, message};
        } catch (e) {
            return {ok: false, message: this.panic || String((e && e.message) || e)};
        }
    }

    /** Start the loaded project (its green flag is clicked). */
    start (scale) {
        this.started = true;
        // Starting follows a click on the green flag, so the browser lets
        // the audio context run.
        this.audio.ensure();
        this.ex.scrust_init(scale, (Math.random() * 4294967296) >>> 0, Date.now() - performance.now(), -new Date().getTimezoneOffset());
        this.raf = requestAnimationFrame(() => this._frame());
    }

    _frame () {
        if (this.stopped) return;
        const ex = this.ex;
        try {
            if (ex.scrust_frame(performance.now())) {
                const width = ex.scrust_width();
                const height = ex.scrust_height();
                // A view of the module's memory; the renderer copies it at once.
                const data = new Uint8Array(ex.memory.buffer, ex.scrust_frame_ptr(), width * height * 4);
                this.post({type: 'frame', width, height, data});
            }
            const threads = ex.scrust_threads();
            if (threads !== this.lastThreads) {
                const finished = threads === 0 && this.lastThreads > 0;
                this.lastThreads = threads;
                this.post({type: 'threads', count: threads});
                // As the embed host does: report the state when the run is over.
                if (finished) this._postState();
            }
        } catch (e) {
            this._exit(e);
            return;
        }
        this.raf = requestAnimationFrame(() => this._frame());
    }

    _postState () {
        const ex = this.ex;
        const len = ex.scrust_state();
        this.post({type: 'state', json: decoder.decode(new Uint8Array(ex.memory.buffer, ex.scrust_state_ptr(), len))});
    }

    _exit (error) {
        this.kill();
        this.post({type: 'exit', code: null, signal: 'trap', log: this.panic || String((error && error.message) || error)});
    }

    /** One line of the embed protocol. */
    input (line) {
        if (this.stopped || !this.ex) return;
        const ex = this.ex;
        const [cmd, a, b, c] = line.split(' ');
        try {
            switch (cmd) {
            case 'move':
                ex.scrust_mouse_move(Number(a), Number(b));
                break;
            case 'button':
                this.audio.ensure();
                ex.scrust_mouse_button(Number(a), Number(b), c === '1' ? 1 : 0);
                break;
            case 'key': {
                if (a === '1') this.audio.ensure();
                const bytes = encoder.encode(unhex(b || ''));
                const ptr = ex.scrust_buffer(bytes.length);
                new Uint8Array(ex.memory.buffer, ptr, bytes.length).set(bytes);
                ex.scrust_key(bytes.length, a === '1' ? 1 : 0);
                break;
            }
            case 'flag':
                this.audio.ensure();
                ex.scrust_green_flag();
                break;
            case 'stop':
                ex.scrust_stop();
                break;
            case 'state':
                this._postState();
                break;
            }
        } catch (e) {
            this._exit(e);
        }
    }

    kill () {
        if (this.stopped) return;
        this.stopped = true;
        cancelAnimationFrame(this.raf);
        this.audio.close();
    }
}

/**
 * The green flag's "compiler": scrust's interpreter running in this page.
 * @param {string} url of scrust-interp.wasm
 */
function createInterpreter (url) {
    let modulePromise = null;
    const getModule = () => {
        if (!modulePromise) {
            modulePromise = fetchBytes(url).then(bytes => WebAssembly.compile(bytes));
            modulePromise.catch(() => {
                modulePromise = null;
            });
        }
        return modulePromise;
    };
    let project = null;
    /** Loaded by the last compile and not run yet. */
    let prepared = null;
    let program = null;
    const listeners = [];
    const post = message => listeners.forEach(cb => cb(message));

    const stop = () => {
        if (program) program.kill();
        program = null;
    };

    const newRun = () => {
        const me = new WasmProgram(null, message => {
            if (program === me) post(message);
        });
        return me;
    };

    return {
        busyText: 'プロジェクトを読み込み中…',
        /**
         * Load the project into the interpreter (it reports unsupported blocks).
         * @param {Uint8Array} sb3 the project
         * @param {{fps: number, turbo: boolean}} options
         * @returns {Promise<{ok: boolean, warnings?: string, log: string}>}
         */
        async compile (sb3, options) {
            stop();
            project = null;
            prepared = null;
            let module;
            try {
                module = await getModule();
            } catch (e) {
                return {ok: false, log: `cannot load the scrust interpreter (${url}): ${e.message || e}`};
            }
            const run = newRun();
            run.module = module;
            const result = await run.load(sb3, options);
            if (!result.ok) return {ok: false, log: result.message || 'failed to load the project'};
            project = {sb3, options};
            prepared = run;
            return {ok: true, warnings: result.message, log: ''};
        },
        /** Start the last loaded project (it receives the green flag); resolves to {framesPath: null}. */
        async start (options) {
            stop();
            if (!project) throw new Error('no compiled program');
            let run = prepared;
            prepared = null;
            if (!run) {
                // Every run gets a fresh instance (a run's data lives as long as it).
                run = newRun();
                run.module = await getModule();
                const result = await run.load(project.sb3, project.options);
                if (!result.ok) throw new Error(result.message);
            }
            program = run;
            run.start((options && options.scale) || 1);
            return {framesPath: null};
        },
        send: line => {
            if (program) program.input(line);
        },
        kill: stop,
        onMessage: callback => {
            listeners.push(callback);
        }
    };
}

module.exports = {loadScene, createInterpreter};
