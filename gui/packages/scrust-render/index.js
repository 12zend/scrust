/**
 * Drop-in replacement for scratch-render's RenderWebGL, backed by scrust's
 * CPU compositor (crates/scrust-render) through a native Node-API addon
 * (crates/scrust-node, loaded by the Electron preload as
 * `window.scrustNative.scene`).
 *
 * It has two modes:
 * - editor: the GUI's scratch-vm model creates skins and drawables as it would
 *   with scratch-render; the scrust scene composites them natively and the
 *   result is blitted to a 2D canvas whenever something changed.
 * - program: a compiled scrust program is running (as a native process); its
 *   frames, rendered by the program's own SoftRenderer, are shown as they arrive.
 *
 * Only what the editor needs is implemented. Collision queries and pen
 * drawing belong to running projects, which never run on scratch-vm here.
 */

const EventEmitter = require('events');

const native = typeof window !== 'undefined' && window.scrustNative ? window.scrustNative.scene : null;

/** Resolves when the native scene can be used; the app waits for it before starting. */
const ready = native && native.draw ?
    Promise.resolve() :
    Promise.reject(new Error(native && native.loadError ? native.loadError() :
        'the scrust renderer addon is not available (start the GUI from Electron: gui/)'));
ready.catch(() => {});

const ids = list => new Uint32Array(list);

const toClamped = bytes => new Uint8ClampedArray(bytes.buffer, bytes.byteOffset, bytes.byteLength);

/** Straight RGBA pixels of anything scratch-vm hands to createBitmapSkin. */
const toImageData = source => {
    if (typeof ImageData !== 'undefined' && source instanceof ImageData) return source;
    const width = source.naturalWidth || source.videoWidth || source.width || 0;
    const height = source.naturalHeight || source.videoHeight || source.height || 0;
    const canvas = document.createElement('canvas');
    canvas.width = Math.max(1, width);
    canvas.height = Math.max(1, height);
    const ctx = canvas.getContext('2d');
    if (width > 0 && height > 0) ctx.drawImage(source, 0, 0);
    return ctx.getImageData(0, 0, canvas.width, canvas.height);
};

class Bounds {
    constructor (left, right, top, bottom) {
        this.left = left;
        this.right = right;
        this.top = top;
        this.bottom = bottom;
    }
    get width () {
        return this.right - this.left;
    }
    get height () {
        return this.top - this.bottom;
    }
}

class ScrustRender extends EventEmitter {
    constructor (canvas, xLeft = -240, xRight = 240, yBottom = -180, yTop = 180) {
        super();
        if (!native || !native.draw) throw new Error('scrust-render: the native renderer is not available');
        this.canvas = canvas;
        this._ctx = canvas.getContext('2d');
        this._xLeft = xLeft;
        this._xRight = xRight;
        this._yBottom = yBottom;
        this._yTop = yTop;
        this._nativeSize = [Math.abs(xRight - xLeft), Math.abs(yTop - yBottom)];

        this._groupOrdering = [];
        /** @type {Object<string, number[]>} drawable ids per layer group, back to front */
        this._groups = {};
        /** drawable id -> {group, skinId, noninteractive} */
        this._drawables = new Map();
        /** skin id -> {size, center} */
        this._skins = new Map();
        // scratch-vm's interpolation and video sensing peek at this; neither runs here.
        this._allDrawables = [];

        this.dirty = true;
        this.useHighQualityRender = false;
        this.customFonts = {};

        this._mode = 'editor';
        this._programCanvas = null;
        this._lastFrame = null;
        /** The shared frame file being shown ({path, seen}), see watchProgramFrames. */
        this._frameWatch = null;

        this.overlayContainer = document.createElement('div');
        this.overlayContainer.className = 'scratch-render-overlays';
        this._overlays = [];

        this.resize(canvas.clientWidth || this._nativeSize[0], canvas.clientHeight || this._nativeSize[1]);
    }

    static isSupported () {
        return !!(native && native.draw);
    }

    // ---------------------------------------------------------------- canvas

    resize (pixelsWide, pixelsTall) {
        const ratio = window.devicePixelRatio || 1;
        const width = Math.max(1, Math.round(pixelsWide * ratio));
        const height = Math.max(1, Math.round(pixelsTall * ratio));
        if (this.canvas.width === width && this.canvas.height === height && this._sized) return;
        this._sized = true;
        this.canvas.width = width;
        this.canvas.height = height;
        native.resize(width, height);
        this._updateOverlays();
        this.dirty = true;
        this.draw();
    }

    setBackgroundColor (red, green, blue) {
        native.background(Math.round(red * 255), Math.round(green * 255), Math.round(blue * 255));
        this.dirty = true;
    }

    setStageSize (xLeft, xRight, yBottom, yTop) {
        this._xLeft = xLeft;
        this._xRight = xRight;
        this._yBottom = yBottom;
        this._yTop = yTop;
        this._nativeSize = [Math.abs(xRight - xLeft), Math.abs(yTop - yBottom)];
        this._updateOverlays();
        this.emit('NativeSizeChanged', {newSize: this._nativeSize});
    }

    getNativeSize () {
        return [this._nativeSize[0], this._nativeSize[1]];
    }

    setUseHighQualityRender (enabled) {
        this.useHighQualityRender = !!enabled;
        this.emit('UseHighQualityRenderChanged', this.useHighQualityRender);
    }

    setCustomFonts (fonts) {
        this.customFonts = fonts;
    }

    setPrivateSkinAccess () {}
    setMaxTextureDimension () {}
    setDebugCanvas () {}
    setUseGpuMode () {}
    markSkinAsPrivate () {}

    addOverlay (element, mode = 'scale') {
        const container = document.createElement('div');
        container.appendChild(element);
        this.overlayContainer.appendChild(container);
        const overlay = {container, userElement: element, mode};
        this._overlays.push(overlay);
        this._updateOverlays();
        return overlay;
    }

    removeOverlay (element) {
        const i = this._overlays.findIndex(o => o.userElement === element);
        if (i !== -1) {
            this._overlays[i].container.remove();
            this._overlays.splice(i, 1);
        }
    }

    _updateOverlays () {
        const ratio = window.devicePixelRatio || 1;
        const width = this.canvas.width / ratio;
        const height = this.canvas.height / ratio;
        this.overlayContainer.style.width = `${width}px`;
        this.overlayContainer.style.height = `${height}px`;
        const [nw, nh] = this._nativeSize;
        for (const {container, mode} of this._overlays) {
            if (mode === 'scale' || mode === 'scale-centered') {
                container.style.width = `${nw}px`;
                container.style.height = `${nh}px`;
                container.style.transformOrigin = 'top left';
                const scale = `scale(${width / nw}, ${height / nh})`;
                container.style.transform = mode === 'scale' ? scale : `${scale} translate(${nw / 2}px, ${nh / 2}px)`;
            } else {
                container.style.transform = '';
                container.style.width = '100%';
                container.style.height = '100%';
            }
        }
    }

    // ---------------------------------------------------------------- skins

    _skinInfo (skinId) {
        const v = native.skinInfo(skinId);
        if (!v) return {size: [0, 0], center: [0, 0]};
        const [w, h, cx, cy] = v;
        const info = {size: [w, h], center: [cx, cy]};
        this._skins.set(skinId, info);
        return info;
    }

    _svgSkin (id, svgData, rotationCenter) {
        const has = Array.isArray(rotationCenter);
        const skinId = native.svgSkin(id, String(svgData), has, has ? rotationCenter[0] : 0, has ? rotationCenter[1] : 0);
        this._skinInfo(skinId);
        this.dirty = true;
        return skinId;
    }

    _bitmapSkin (id, bitmapData, resolution, rotationCenter) {
        const image = toImageData(bitmapData);
        const has = Array.isArray(rotationCenter);
        const skinId = native.bitmapSkin(
            id, image.width, image.height, image.data, resolution || 1,
            has, has ? rotationCenter[0] : 0, has ? rotationCenter[1] : 0
        );
        this._skinInfo(skinId);
        this.dirty = true;
        return skinId;
    }

    createSVGSkin (svgData, rotationCenter) {
        return this._svgSkin(0, svgData, rotationCenter);
    }

    updateSVGSkin (skinId, svgData, rotationCenter) {
        this._svgSkin(skinId, svgData, rotationCenter);
    }

    createBitmapSkin (bitmapData, costumeResolution, rotationCenter) {
        return this._bitmapSkin(0, bitmapData, costumeResolution, rotationCenter);
    }

    updateBitmapSkin (skinId, bitmapData, costumeResolution, rotationCenter) {
        this._bitmapSkin(skinId, bitmapData, costumeResolution, rotationCenter);
    }

    // Pen and speech bubbles are drawn by the compiled program, not here.
    createPenSkin () {
        const id = native.emptySkin();
        this._skinInfo(id);
        return id;
    }

    createTextSkin () {
        return this.createPenSkin();
    }

    updateTextSkin () {}

    destroySkin (skinId) {
        native.destroySkin(skinId);
        this._skins.delete(skinId);
        this.dirty = true;
    }

    getSkinSize (skinId) {
        const info = this._skins.get(skinId) || this._skinInfo(skinId);
        return [info.size[0], info.size[1]];
    }

    getSkinRotationCenter (skinId) {
        const info = this._skins.get(skinId) || this._skinInfo(skinId);
        return [info.center[0], info.center[1]];
    }

    getCurrentSkinSize (drawableID) {
        const d = this._drawables.get(drawableID);
        return d && d.skinId ? this.getSkinSize(d.skinId) : [0, 0];
    }

    // ---------------------------------------------------------------- layers

    setLayerGroupOrdering (groupOrdering) {
        this._groupOrdering = groupOrdering.slice();
        for (const g of groupOrdering) {
            if (!this._groups[g]) this._groups[g] = [];
        }
    }

    _drawList () {
        const list = [];
        for (const g of this._groupOrdering) list.push(...this._groups[g]);
        return list;
    }

    /** Index in the full draw list at which `group` starts. */
    _groupStart (group) {
        let start = 0;
        for (const g of this._groupOrdering) {
            if (g === group) break;
            start += this._groups[g].length;
        }
        return start;
    }

    createDrawable (group) {
        if (!this._groups[group]) {
            this._groups[group] = [];
            this._groupOrdering.push(group);
        }
        const id = native.createDrawable();
        this._drawables.set(id, {group, skinId: 0, noninteractive: false});
        this._groups[group].push(id);
        this.dirty = true;
        return id;
    }

    destroyDrawable (drawableID, group) {
        const d = this._drawables.get(drawableID);
        const list = this._groups[group || (d && d.group)];
        if (list) {
            const i = list.indexOf(drawableID);
            if (i !== -1) list.splice(i, 1);
        }
        this._drawables.delete(drawableID);
        native.destroyDrawable(drawableID);
        this.dirty = true;
    }

    markDrawableAsNoninteractive (drawableID) {
        const d = this._drawables.get(drawableID);
        if (d) d.noninteractive = true;
    }

    getDrawableOrder (drawableID) {
        return this._drawList().indexOf(drawableID);
    }

    /**
     * Move a drawable within its layer group. `order` is an index in the full
     * draw list, or an offset from the current one when `optIsRelative`;
     * `optMin` is the lowest index within the group it may take. Returns the
     * new index in the full draw list, or null if the drawable is not in the
     * group.
     */
    setDrawableOrder (drawableID, order, group, optIsRelative, optMin) {
        const list = this._groups[group];
        if (!list) return null;
        const local = list.indexOf(drawableID);
        if (local === -1) return null;
        const start = this._groupStart(group);
        const current = start + local;
        if (order === 0 && !optIsRelative) return current;
        list.splice(local, 1);
        let target = optIsRelative ? current + order : order;
        const minLocal = (optMin || 0) < list.length + 1 && (optMin || 0) >= 0 ? (optMin || 0) : 0;
        target = Math.min(Math.max(target - start, minLocal), list.length);
        list.splice(target, 0, drawableID);
        this.dirty = true;
        return start + target;
    }

    // ---------------------------------------------------------------- drawable state

    updateDrawableSkinId (drawableID, skinId) {
        const d = this._drawables.get(drawableID);
        if (d) d.skinId = skinId;
        native.drawableSkin(drawableID, skinId);
        this.dirty = true;
    }

    updateDrawablePosition (drawableID, position) {
        native.drawablePosition(drawableID, position[0], position[1]);
        this.dirty = true;
    }

    updateDrawableDirection (drawableID, direction) {
        native.drawableDirection(drawableID, direction);
        this.dirty = true;
    }

    updateDrawableScale (drawableID, scale) {
        native.drawableScale(drawableID, scale[0], scale[1]);
        this.dirty = true;
    }

    updateDrawableDirectionScale (drawableID, direction, scale) {
        this.updateDrawableDirection(drawableID, direction);
        this.updateDrawableScale(drawableID, scale);
    }

    updateDrawableVisible (drawableID, visible) {
        native.drawableVisible(drawableID, !!visible);
        this.dirty = true;
    }

    updateDrawableEffect (drawableID, effectName, value) {
        native.drawableEffect(drawableID, effectName, value);
        this.dirty = true;
    }

    updateDrawableProperties (drawableID, properties) {
        if ('skinId' in properties) this.updateDrawableSkinId(drawableID, properties.skinId);
        if ('position' in properties) this.updateDrawablePosition(drawableID, properties.position);
        if ('direction' in properties) this.updateDrawableDirection(drawableID, properties.direction);
        if ('scale' in properties) this.updateDrawableScale(drawableID, properties.scale);
        if ('visible' in properties) this.updateDrawableVisible(drawableID, properties.visible);
        for (const name of ['color', 'fisheye', 'whirl', 'pixelate', 'mosaic', 'brightness', 'ghost']) {
            if (name in properties) this.updateDrawableEffect(drawableID, name, properties[name]);
        }
    }

    getFencedPositionOfDrawable (drawableID, position) {
        return native.fenced(drawableID, position[0], position[1]);
    }

    getBounds (drawableID) {
        const v = native.bounds(drawableID, true);
        if (!v) return null;
        const [l, r, t, b] = v;
        return new Bounds(l, r, t, b);
    }

    getBoundsForBubble (drawableID) {
        return this.getBounds(drawableID);
    }

    // Collision queries only matter to running scripts, which run in the
    // compiled program.
    isTouchingColor () {
        return false;
    }

    isTouchingDrawables () {
        return false;
    }

    drawableTouching () {
        return false;
    }

    offscreenTouching () {
        return false;
    }

    penClear () {}
    penPoint () {}
    penLine () {}
    penStamp () {}

    // ---------------------------------------------------------------- picking

    _clientToScratch (x, y) {
        const w = this.canvas.clientWidth || this._nativeSize[0];
        const h = this.canvas.clientHeight || this._nativeSize[1];
        return [this._nativeSize[0] * ((x / w) - 0.5), this._nativeSize[1] * (0.5 - (y / h))];
    }

    /** Topmost interactive sprite drawable at a client-space point, or -1. */
    pick (centerX, centerY, touchWidth, touchHeight, candidateIDs) {
        if (this._mode === 'program') return -1;
        const sprites = this._groups.sprite || [];
        const candidates = (candidateIDs || sprites).filter(id => {
            const d = this._drawables.get(id);
            return d && !d.noninteractive && sprites.includes(id);
        });
        if (candidates.length === 0) return -1;
        const [x, y] = this._clientToScratch(centerX, centerY);
        const id = native.pick(ids(candidates), x, y);
        return id === 0 ? -1 : id;
    }

    /** The drawable rendered alone, for the GUI's sprite drag image. */
    extractDrawableScreenSpace (drawableID) {
        const v = native.extract(drawableID);
        if (!v) throw new Error(`scrust-render: no drawable ${drawableID}`);
        const [x, y, w, h, data] = v;
        const pixels = toClamped(data);
        const ratio = this.canvas.width / (this.canvas.clientWidth || this.canvas.width);
        return {
            imageData: new ImageData(pixels, w, h),
            x: x / ratio,
            y: y / ratio,
            width: w / ratio,
            height: h / ratio
        };
    }

    /** Colour under a client-space point and the stage pixels around it (colour picker loupe). */
    extractColor (x, y, radius) {
        this.draw();
        const size = (radius * 2) + 1;
        const data = new Uint8Array(size * size * 4);
        const sx = this.canvas.width / (this.canvas.clientWidth || this.canvas.width);
        const unit = this.canvas.width / this._nativeSize[0];
        const frame = this._ctx.getImageData(0, 0, this.canvas.width, this.canvas.height).data;
        const at = (px, py) => {
            const cx = Math.min(this.canvas.width - 1, Math.max(0, Math.floor(px)));
            const cy = Math.min(this.canvas.height - 1, Math.max(0, Math.floor(py)));
            return (cy * this.canvas.width + cx) * 4;
        };
        for (let j = 0; j < size; j++) {
            for (let i = 0; i < size; i++) {
                const k = at((x * sx) + ((i - radius) * unit), (y * sx) + ((j - radius) * unit));
                data.set(frame.subarray(k, k + 4), ((j * size) + i) * 4);
            }
        }
        const c = ((radius * size) + radius) * 4;
        return {data, width: size, height: size, color: {r: data[c], g: data[c + 1], b: data[c + 2], a: 255}};
    }

    // ---------------------------------------------------------------- drawing

    draw () {
        if (this._mode === 'program') {
            if (this.dirty && this._programCanvas) this._blitProgram();
            this.dirty = false;
            return;
        }
        if (!this.dirty) return;
        this.dirty = false;
        const pixels = toClamped(native.draw(ids(this._drawList())));
        const {width, height} = this.canvas;
        if (pixels.length === width * height * 4) this._ctx.putImageData(new ImageData(pixels, width, height), 0, 0);
    }

    requestSnapshot (callback) {
        this.draw();
        callback(this.canvas.toDataURL());
    }

    // ---------------------------------------------------------------- program mode

    /** Switch between showing the editor scene and a running program's frames. */
    setProgramMode (enabled) {
        this._mode = enabled ? 'program' : 'editor';
        if (!enabled) {
            this._programCanvas = null;
            this._frameWatch = null;
        }
        this.dirty = true;
        this.draw();
    }

    get programMode () {
        return this._mode === 'program';
    }

    /** Whether a program's frames can be read from shared memory (watchProgramFrames). */
    get canReadProgramFrames () {
        return !!(native && native.frame);
    }

    /**
     * Show the frames a program publishes in the shared memory file `path`
     * (crates/scrust-render/src/frames.rs), checked once per animation frame,
     * until another file is watched or program mode ends. Frames that
     * arrive between two checks are skipped.
     * @param {string} path the file passed to the program as --frames-shm
     */
    watchProgramFrames (path) {
        const watch = {path, seen: 0};
        this._frameWatch = watch;
        const poll = () => {
            if (this._frameWatch !== watch) return;
            const frame = native.frame(path, watch.seen);
            if (frame) {
                const [number, width, height, rgba] = frame;
                watch.seen = number;
                this.showProgramFrame(rgba, width, height);
            }
            requestAnimationFrame(poll);
        };
        requestAnimationFrame(poll);
    }

    /**
     * Show a frame (straight RGBA bytes, opaque) rendered by the compiled
     * program, or the canvas it drew the frame in (the browser build's
     * WebGL renderer; copied without leaving the GPU).
     */
    showProgramFrame (rgba, width, height, canvas) {
        if (canvas) {
            this._programCanvas = canvas;
            this._blitProgram();
            return;
        }
        if (!this._programCanvas || this._programCanvas.getContext('2d') === null || this._programCanvas.width !== width || this._programCanvas.height !== height) {
            this._programCanvas = document.createElement('canvas');
            this._programCanvas.width = width;
            this._programCanvas.height = height;
        }
        this._programCanvas.getContext('2d').putImageData(new ImageData(toClamped(rgba), width, height), 0, 0);
        this._blitProgram();
    }

    _blitProgram () {
        const ctx = this._ctx;
        ctx.imageSmoothingEnabled = true;
        ctx.drawImage(this._programCanvas, 0, 0, this.canvas.width, this.canvas.height);
    }
}

ScrustRender.ready = ready;

module.exports = ScrustRender;
