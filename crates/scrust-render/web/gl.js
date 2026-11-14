/**
 * WebGL2 side of scrust_render's browser compositor (crates/scrust-render/src/webgl.rs):
 * the `scrust_gl` imports of a WebAssembly module that contains a renderer.
 * The module records each frame as a stream of floats, which `gl_run`
 * replays here; the shaders are those of the Metal compositor (gpu.rs),
 * themselves scratch-render's.
 *
 * Used by the GUI's browser build (gui/web/bridge.js) and by the standalone
 * page of programs built with `--target wasm` (crates/scrust-player/web/index.html,
 * which inlines this file).
 *
 *   const gl = createScrustGl(() => instance.exports.memory.buffer);
 *   WebAssembly.instantiate(module, {env, scrust_gl: gl.imports});
 *   // after the module's renderer started: gl.canvas holds the frames
 *
 * Without WebGL2, `gl_init` returns 0 and the module draws on the CPU.
 */
/* eslint-disable no-bitwise */
// eslint-disable-next-line no-unused-vars
function createScrustGl (memory, options) {
    const opts = options || {};
    const state = {canvas: null, gl: null};

    const VS_SPRITE = `#version 300 es
uniform vec4 u_verts[6];
out vec2 v_uv;
void main () {
    vec4 v = u_verts[gl_VertexID];
    gl_Position = vec4(v.xy, 0.0, 1.0);
    v_uv = v.zw;
}`;

    const FS_SPRITE = `#version 300 es
precision highp float;
// size: skin size (stage units), distort, recolor;
// fxa: color, fisheye, whirl, pixelate; fxb: mosaic, brightness, ghost.
uniform vec4 u_size;
uniform vec4 u_fxa;
uniform vec4 u_fxb;
uniform sampler2D u_tex;
in vec2 v_uv;
out vec4 o_color;

vec2 effect_point (vec2 uv) {
    float mosaic = u_fxb.x, pixelate = u_fxa.w, whirl = u_fxa.z, fisheye = u_fxa.y;
    if (mosaic != 0.0) {
        float m = clamp(round((abs(mosaic) + 10.0) / 10.0), 1.0, 512.0);
        uv = fract(m * uv);
    }
    if (pixelate != 0.0) {
        float p = abs(pixelate) / 10.0;
        vec2 t = u_size.xy / p;
        uv = (floor(uv * t) + 0.5) / t;
    }
    if (whirl != 0.0) {
        vec2 o = uv - 0.5;
        float mag = length(o);
        float factor = max(1.0 - mag / 0.5, 0.0);
        float a = (-whirl * 3.141592653589793 / 180.0) * factor * factor;
        float sn = sin(a), cs = cos(a);
        uv = vec2(cs * o.x + sn * o.y + 0.5, -sn * o.x + cs * o.y + 0.5);
    }
    if (fisheye != 0.0) {
        vec2 v = (uv - 0.5) / 0.5;
        float len = length(v);
        if (len > 0.0) {
            float power = max((fisheye + 100.0) / 100.0, 0.0);
            float r = pow(min(len, 1.0), power) * max(len, 1.0);
            uv = 0.5 + r * (v / len) * 0.5;
        }
    }
    return uv;
}

vec3 rgb_to_hsv (vec3 c) {
    float mx = max(max(c.r, c.g), c.b), mn = min(min(c.r, c.g), c.b), d = mx - mn;
    float h = 0.0;
    if (d != 0.0) {
        if (mx == c.r) { float q = (c.g - c.b) / d; h = (q - 6.0 * floor(q / 6.0)) / 6.0; }
        else if (mx == c.g) h = ((c.b - c.r) / d + 2.0) / 6.0;
        else h = ((c.r - c.g) / d + 4.0) / 6.0;
    }
    return vec3(h, mx == 0.0 ? 0.0 : d / mx, mx);
}

vec3 hsv_to_rgb (vec3 c) {
    float i = floor(c.x * 6.0), f = c.x * 6.0 - i, v = c.z, s = c.y;
    float p = v * (1.0 - s), q = v * (1.0 - f * s), t = v * (1.0 - (1.0 - f) * s);
    int k = int(i - 6.0 * floor(i / 6.0));
    if (k == 0) return vec3(v, t, p);
    if (k == 1) return vec3(q, v, p);
    if (k == 2) return vec3(p, v, t);
    if (k == 3) return vec3(p, q, v);
    if (k == 4) return vec3(t, p, v);
    return vec3(v, p, q);
}

vec4 effect_color (vec4 c) {
    float a = c.a;
    vec3 rgb = c.rgb / a;
    float color = u_fxa.x, brightness = u_fxb.y, ghost = u_fxb.z;
    if (color != 0.0) {
        vec3 hsv = rgb_to_hsv(rgb);
        const float MIN_V = 0.11 / 2.0, MIN_S = 0.09;
        if (hsv.z < MIN_V) hsv = vec3(0.0, 1.0, MIN_V);
        else if (hsv.y < MIN_S) hsv = vec3(0.0, MIN_S, hsv.z);
        hsv.x = fract(hsv.x + color / 200.0);
        rgb = hsv_to_rgb(hsv);
    }
    if (brightness != 0.0) rgb = clamp(rgb + brightness / 100.0, 0.0, 1.0);
    if (ghost != 0.0) a *= clamp(1.0 - ghost / 100.0, 0.0, 1.0);
    return vec4(rgb * a, a);
}

void main () {
    vec2 uv = v_uv;
    if (u_size.z != 0.0) {
        uv = effect_point(uv);
        if (uv.x < 0.0 || uv.y < 0.0 || uv.x >= 1.0 || uv.y >= 1.0) discard;
    }
    vec4 c = texture(u_tex, uv);
    if (c.a <= 0.0) discard;
    if (u_size.w != 0.0) c = effect_color(c);
    o_color = c;
}`;

    // Screen-aligned textured rectangle: x0, y at v = 0, x1, y at v = 1 (NDC).
    const VS_RECT = `#version 300 es
uniform vec4 u_rect;
out vec2 v_uv;
void main () {
    vec2 uv = vec2(float(gl_VertexID & 1), float(gl_VertexID >> 1));
    gl_Position = vec4(mix(u_rect.x, u_rect.z, uv.x), mix(u_rect.y, u_rect.w, uv.y), 0.0, 1.0);
    v_uv = uv;
}`;

    const FS_RECT = `#version 300 es
precision highp float;
uniform sampler2D u_tex;
in vec2 v_uv;
out vec4 o_color;
void main () {
    o_color = texture(u_tex, v_uv);
}`;

    // scratch-render's pen line (sprite.vert / sprite.frag, DRAW_MODE_line),
    // one instance per line: premultiplied colour, (thickness, length,
    // stage size), pen points (x0, -y0, dx, -dy) in device pixels.
    const VS_LINE = `#version 300 es
in vec4 a_color;
in vec4 a_tl;
in vec4 a_pts;
out vec2 v_tc;
out vec4 v_color;
out float v_thickness;
out float v_length;
void main () {
    const float epsilon = 1e-3;
    vec2 a_position = vec2(float((gl_VertexID & 1) == 0), float(gl_VertexID >> 1));
    vec2 position = a_position;
    float expandedRadius = (a_tl.x * 0.5) + 1.4142135623730951;
    v_tc.x = mix(0.0, a_tl.y + (expandedRadius * 2.0), a_position.x) - expandedRadius;
    v_tc.y = ((a_position.y - 0.5) * expandedRadius) + 0.5;
    position.x *= a_tl.y + (2.0 * expandedRadius);
    position.y *= 2.0 * expandedRadius;
    position -= expandedRadius;
    vec2 pointDiff = a_pts.zw;
    pointDiff.x = (abs(pointDiff.x) < epsilon && abs(pointDiff.y) < epsilon) ? epsilon : pointDiff.x;
    vec2 normalized = pointDiff / max(a_tl.y, epsilon);
    position = mat2(normalized.x, normalized.y, -normalized.y, normalized.x) * position;
    position += a_pts.xy;
    position *= 2.0 / a_tl.zw;
    gl_Position = vec4(position, 0.0, 1.0);
    v_color = a_color;
    v_thickness = a_tl.x;
    v_length = a_tl.y;
}`;

    const FS_LINE = `#version 300 es
precision highp float;
in vec2 v_tc;
in vec4 v_color;
in float v_thickness;
in float v_length;
out vec4 o_color;
void main () {
    float d = ((v_tc.x - clamp(v_tc.x, 0.0, v_length)) * 0.5) + 0.5;
    float line = distance(vec2(0.5), vec2(d, v_tc.y)) * 2.0;
    line -= ((v_thickness - 1.0) * 0.5);
    o_color = v_color * clamp(1.0 - line, 0.0, 1.0);
}`;

    const compile = (gl, vs, fs, names) => {
        const shader = (type, src) => {
            const s = gl.createShader(type);
            gl.shaderSource(s, src);
            gl.compileShader(s);
            if (!gl.getShaderParameter(s, gl.COMPILE_STATUS)) throw new Error(gl.getShaderInfoLog(s));
            return s;
        };
        const p = gl.createProgram();
        gl.attachShader(p, shader(gl.VERTEX_SHADER, vs));
        gl.attachShader(p, shader(gl.FRAGMENT_SHADER, fs));
        gl.bindAttribLocation(p, 0, 'a_color');
        gl.bindAttribLocation(p, 1, 'a_tl');
        gl.bindAttribLocation(p, 2, 'a_pts');
        gl.linkProgram(p);
        if (!gl.getProgramParameter(p, gl.LINK_STATUS)) throw new Error(gl.getProgramInfoLog(p));
        const u = {program: p};
        for (const n of names) u[n] = gl.getUniformLocation(p, n);
        return u;
    };

    const newTexture = (gl, w, h) => {
        const t = gl.createTexture();
        gl.bindTexture(gl.TEXTURE_2D, t);
        gl.texImage2D(gl.TEXTURE_2D, 0, gl.RGBA8, w, h, 0, gl.RGBA, gl.UNSIGNED_BYTE, null);
        return t;
    };

    /** The pen layer: texture and framebuffer. */
    const newTarget = (gl, w, h) => {
        const tex = newTexture(gl, w, h);
        const fb = gl.createFramebuffer();
        gl.bindFramebuffer(gl.FRAMEBUFFER, fb);
        gl.framebufferTexture2D(gl.FRAMEBUFFER, gl.COLOR_ATTACHMENT0, gl.TEXTURE_2D, tex, 0);
        gl.clearColor(0, 0, 0, 0);
        gl.clear(gl.COLOR_BUFFER_BIT);
        return {tex, fb};
    };

    const setup = (width, height) => {
        const canvas = opts.canvas || (typeof OffscreenCanvas !== 'undefined' && opts.offscreen ?
            new OffscreenCanvas(width, height) :
            document.createElement('canvas'));
        canvas.width = width;
        canvas.height = height;
        const gl = canvas.getContext('webgl2', {
            alpha: false,
            antialias: false,
            depth: false,
            stencil: false,
            premultipliedAlpha: true,
            // The GUI copies the canvas onto the stage, possibly later than
            // the task that drew it.
            preserveDrawingBuffer: true
        });
        if (!gl) return false;
        const s = {
            canvas,
            gl,
            width,
            height,
            sprite: compile(gl, VS_SPRITE, FS_SPRITE, ['u_verts', 'u_size', 'u_fxa', 'u_fxb', 'u_tex']),
            rect: compile(gl, VS_RECT, FS_RECT, ['u_rect', 'u_tex']),
            line: compile(gl, VS_LINE, FS_LINE, []),
            textures: new Map(),
            emptyVao: gl.createVertexArray(),
            lineVao: gl.createVertexArray(),
            lineBuffer: gl.createBuffer(),
            nearest: gl.createSampler(),
            linear: gl.createSampler(),
            pen: null,
            current: null
        };
        for (const [smp, filter] of [[s.nearest, gl.NEAREST], [s.linear, gl.LINEAR]]) {
            gl.samplerParameteri(smp, gl.TEXTURE_MIN_FILTER, filter);
            gl.samplerParameteri(smp, gl.TEXTURE_MAG_FILTER, filter);
            gl.samplerParameteri(smp, gl.TEXTURE_WRAP_S, gl.CLAMP_TO_EDGE);
            gl.samplerParameteri(smp, gl.TEXTURE_WRAP_T, gl.CLAMP_TO_EDGE);
        }
        gl.bindVertexArray(s.lineVao);
        gl.bindBuffer(gl.ARRAY_BUFFER, s.lineBuffer);
        for (let i = 0; i < 3; i++) {
            gl.enableVertexAttribArray(i);
            gl.vertexAttribPointer(i, 4, gl.FLOAT, false, 48, i * 16);
            gl.vertexAttribDivisor(i, 1);
        }
        gl.bindVertexArray(null);
        for (const p of [s.sprite, s.rect]) {
            gl.useProgram(p.program);
            gl.uniform1i(p.u_tex, 0);
        }
        gl.pixelStorei(gl.UNPACK_ALIGNMENT, 1);
        gl.blendFunc(gl.ONE, gl.ONE_MINUS_SRC_ALPHA);
        Object.assign(state, s);
        resize(width, height);
        return true;
    };

    const resize = (width, height) => {
        const gl = state.gl;
        if (state.pen) {
            gl.deleteFramebuffer(state.pen.fb);
            gl.deleteTexture(state.pen.tex);
        }
        state.width = width;
        state.height = height;
        state.canvas.width = width;
        state.canvas.height = height;
        state.pen = newTarget(gl, width, height);
    };

    const use = p => {
        if (state.current !== p) {
            state.gl.useProgram(p.program);
            state.current = p;
        }
    };

    /** A quad at `o` (see webgl.rs): texture, nearest, viewport, uniforms, vertices. */
    const quad = (f, o) => {
        const gl = state.gl;
        const s = state.sprite;
        use(s);
        gl.bindVertexArray(state.emptyVao);
        gl.viewport(f[o + 2], f[o + 3], f[o + 4], f[o + 5]);
        gl.uniform4fv(s.u_size, f, o + 6, 4);
        gl.uniform4fv(s.u_fxa, f, o + 10, 4);
        gl.uniform4fv(s.u_fxb, f, o + 14, 4);
        gl.uniform4fv(s.u_verts, f, o + 18, 24);
        gl.bindTexture(gl.TEXTURE_2D, state.textures.get(f[o]) || null);
        gl.bindSampler(0, f[o + 1] ? state.nearest : state.linear);
        gl.drawArrays(gl.TRIANGLES, 0, 6);
        return o + 42;
    };

    const rect = (tex, x0, y0, x1, y1) => {
        const gl = state.gl;
        const r = state.rect;
        use(r);
        gl.bindVertexArray(state.emptyVao);
        gl.viewport(0, 0, state.width, state.height);
        gl.uniform4f(r.u_rect, x0, y0, x1, y1);
        gl.bindTexture(gl.TEXTURE_2D, tex);
        gl.bindSampler(0, state.nearest);
        gl.drawArrays(gl.TRIANGLE_STRIP, 0, 4);
    };

    const run = (ptr, len) => {
        const gl = state.gl;
        const f = new Float32Array(memory(), ptr, len);
        state.current = null;
        gl.activeTexture(gl.TEXTURE0);
        gl.enable(gl.BLEND);
        let o = 0;
        while (o < len) {
            switch (f[o++]) {
            case 1: // pen clear
                gl.bindFramebuffer(gl.FRAMEBUFFER, state.pen.fb);
                gl.clearColor(0, 0, 0, 0);
                gl.clear(gl.COLOR_BUFFER_BIT);
                break;
            case 2: { // pen lines
                const n = f[o++];
                gl.bindFramebuffer(gl.FRAMEBUFFER, state.pen.fb);
                gl.viewport(0, 0, state.width, state.height);
                use(state.line);
                gl.bindVertexArray(state.lineVao);
                gl.bindBuffer(gl.ARRAY_BUFFER, state.lineBuffer);
                gl.bufferData(gl.ARRAY_BUFFER, f, gl.STREAM_DRAW, o, n * 12);
                gl.drawArraysInstanced(gl.TRIANGLE_STRIP, 0, 4, n);
                o += n * 12;
                break;
            }
            case 3: // pen stamp
                gl.bindFramebuffer(gl.FRAMEBUFFER, state.pen.fb);
                o = quad(f, o);
                break;
            // The stage is drawn straight into the canvas, as scratch-render
            // does: GL implementations orient the default framebuffer and
            // textures differently (ANGLE flips the former), which decides
            // the pixels a quad edge through pixel centres covers.
            case 4: // begin
                gl.bindFramebuffer(gl.FRAMEBUFFER, null);
                gl.viewport(0, 0, state.width, state.height);
                gl.clearColor(f[o], f[o + 1], f[o + 2], 1);
                gl.clear(gl.COLOR_BUFFER_BIT);
                o += 3;
                break;
            case 5: // sprite
                gl.bindFramebuffer(gl.FRAMEBUFFER, null);
                o = quad(f, o);
                break;
            case 6: // overlay
                gl.bindFramebuffer(gl.FRAMEBUFFER, null);
                rect(state.textures.get(f[o]) || null, f[o + 1], f[o + 2], f[o + 3], f[o + 4]);
                o += 5;
                break;
            case 7: // pen layer (top down) over the stage (bottom up)
                gl.bindFramebuffer(gl.FRAMEBUFFER, null);
                rect(state.pen.tex, -1, 1, 1, -1);
                break;
            case 8: // present: the frame is already in the canvas
                break;
            default:
                throw new Error(`scrust_gl: bad command ${f[o - 1]} at ${o - 1}`);
            }
        }
        gl.bindVertexArray(null);
    };

    const imports = {
        gl_init: (width, height) => {
            if (opts.disabled) return 0;
            try {
                return setup(width, height) ? 1 : 0;
            } catch (e) {
                if (typeof console !== 'undefined') console.warn('scrust: WebGL2 unavailable, drawing on the CPU:', e);
                return 0;
            }
        },
        gl_resize: (width, height) => {
            if (!state.gl) return 0;
            resize(width, height);
            return 1;
        },
        gl_texture: (id, w, h, ptr) => {
            const gl = state.gl;
            let t = state.textures.get(id);
            if (!t) {
                t = gl.createTexture();
                state.textures.set(id, t);
            }
            gl.bindTexture(gl.TEXTURE_2D, t);
            gl.texImage2D(gl.TEXTURE_2D, 0, gl.RGBA8, w, h, 0, gl.RGBA, gl.UNSIGNED_BYTE, new Uint8Array(memory(), ptr, w * h * 4));
        },
        gl_delete_texture: id => {
            const t = state.textures.get(id);
            if (t) {
                state.gl.deleteTexture(t);
                state.textures.delete(id);
            }
        },
        gl_run: run,
        gl_read: (which, ptr, len) => {
            const gl = state.gl;
            gl.bindFramebuffer(gl.FRAMEBUFFER, which === 1 ? state.pen.fb : null);
            const n = Math.min(len, state.width * state.height * 4);
            gl.readPixels(0, 0, state.width, state.height, gl.RGBA, gl.UNSIGNED_BYTE, new Uint8Array(memory(), ptr, n));
        }
    };

    return {
        imports,
        /** The canvas frames are presented in (null until the renderer starts or without WebGL2). */
        get canvas () {
            return state.gl ? state.canvas : null;
        },
        /** Free the context (browsers limit how many may be alive). */
        destroy () {
            const gl = state.gl;
            if (!gl) return;
            const ext = gl.getExtension('WEBGL_lose_context');
            if (ext) ext.loseContext();
            state.gl = null;
        }
    };
}

if (typeof module !== 'undefined' && module.exports) module.exports = createScrustGl;
