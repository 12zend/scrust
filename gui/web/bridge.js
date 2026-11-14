/**
 * Browser counterparts of what the Electron preload (gui/electron/preload.js)
 * gives the page, with the same interfaces, so gui/packages/scrust-vm and
 * gui/packages/scrust-render run unchanged:
 *
 * - `scrustNative.scene`: the editor's renderer, scrust_render::Scene built
 *   for wasm32 (crates/scrust-scene-wasm) instead of the Node-API addon;
 * - `scrustCompiler`: the green flag posts the project to the compile server
 *   (gui/web/server.js), which returns the program built for wasm32; it runs
 *   here, in the page, driven once per animation frame, instead of as a
 *   native child process. Input arrives as the embed protocol's text lines
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

/** A compiled program running in this page. */
class WasmProgram {
    constructor (module, post) {
        this.module = module;
        this.post = post;
        this.ex = null;
        this.audio = new ProgramAudio();
        this.panic = '';
        this.lastThreads = null;
        this.raf = 0;
        this.stopped = false;
    }

    async start (scale) {
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
        this.ex = instance.exports;
        audio.ex = this.ex;
        // Starting the program is the green flag: it starts on a click, so
        // the browser lets the audio context run.
        audio.ensure();
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
 * @param {string} compileUrl the compile server's /api/compile
 */
function createCompiler (compileUrl) {
    let module = null;
    let program = null;
    const listeners = [];
    const post = message => listeners.forEach(cb => cb(message));

    const stop = () => {
        if (program) program.kill();
        program = null;
    };

    return {
        /**
         * Compile the project to WebAssembly (on the compile server).
         * @param {Uint8Array} sb3 the project
         * @param {{fps: number, turbo: boolean}} options
         * @returns {Promise<{ok: boolean, warnings?: string, log: string}>}
         */
        async compile (sb3, options) {
            stop();
            module = null;
            const query = `fps=${encodeURIComponent(options.fps || 30)}&turbo=${options.turbo ? 1 : 0}`;
            let res;
            try {
                res = await fetch(`${compileUrl}?${query}`, {
                    method: 'POST',
                    headers: {'Content-Type': 'application/x.scratch.sb3'},
                    body: sb3
                });
            } catch (e) {
                return {ok: false, log: `コンパイルサーバー (${compileUrl}) に接続できません: ${e.message || e}`};
            }
            if (!res.ok) return {ok: false, log: (await res.text()) || `HTTP ${res.status}`};
            try {
                module = await WebAssembly.compile(await res.arrayBuffer());
            } catch (e) {
                return {ok: false, log: `invalid WebAssembly from the compile server: ${e.message || e}`};
            }
            return {ok: true, warnings: decodeURIComponent(res.headers.get('X-Scrust-Warnings') || ''), log: ''};
        },
        /** Start the last compiled program (it receives the green flag); resolves to {framesPath: null}. */
        async start (options) {
            stop();
            if (!module) throw new Error('no compiled program');
            const me = new WasmProgram(module, message => {
                if (program === me) post(message);
            });
            program = me;
            await me.start((options && options.scale) || 1);
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

module.exports = {loadScene, createCompiler};
