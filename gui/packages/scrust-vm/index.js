/**
 * Replacement for `scratch-vm` in the GUI.
 *
 * scrust is a compiler and player; it has no editor model (PRODUCT.md §1.3).
 * The editor model the GUI is written against - block containers fed by
 * Blockly events, sprites and costumes, the extension/toolbox metadata and
 * sb3 serialisation - is still provided by scratch-vm's classes, but nothing
 * runs on scratch-vm's interpreter or TurboWarp's JS compiler: threads are
 * never created or stepped. Instead:
 *
 * - the green flag saves the project as sb3 and has scrust compile it to Rust
 *   and then to a native executable (in the Electron main process, see
 *   gui/electron), which runs as a child process; its frames are drawn on
 *   the stage canvas;
 * - stage input (mouse, keyboard) goes to the running program;
 * - when the program stops or goes idle, the state it left behind (positions,
 *   costumes, variables, lists, monitor visibility) is copied back into the
 *   editor model, as Scratch leaves it after a run.
 */

const VirtualMachine = require('scratch-vm/src/virtual-machine.js');
const ProgramHost = require('./program-host');
const StatusOverlay = require('./status');

const COMPILE_HINT = 'Rust にコンパイル中…';

/** Decode a value of a state snapshot (non-finite numbers are tagged). */
const decodeValue = v => (v !== null && typeof v === 'object' ? Number(v.n) : v);

const sameList = (a, b) => Array.isArray(a) && Array.isArray(b) && a.length === b.length && a.every((x, i) => x === b[i]);

class ScrustSession {
    constructor (vm) {
        this.vm = vm;
        this.host = null;
        this.hostKey = null;
        this.busy = false;
        this.cancelled = false;
        /** Options of the executable the main process holds, if any. */
        this.compiledKey = null;
        /** The editor model changed after the last compile started. */
        this.changed = true;
        this.syncing = false;
        this.status = new StatusOverlay(() => this.renderer && this.renderer.overlayContainer);

        vm.runtime.on('PROJECT_CHANGED', () => this._onProjectChanged());
        vm.runtime.on('BEFORE_EXECUTE', () => this._refreshMonitors());
    }

    get renderer () {
        return this.vm.runtime.renderer;
    }

    get live () {
        return this.host !== null;
    }

    threadCount () {
        return this.host ? this.host.threadCount() : 0;
    }

    /** Compile options; an executable built with other options is not reused. */
    _options () {
        const rt = this.vm.runtime;
        const fps = rt.frameLoop.framerate;
        return {fps: fps > 0 ? fps : 60, turbo: !!rt.turboMode};
    }

    async greenFlag () {
        if (this.busy) return;
        const options = this._options();
        const key = JSON.stringify(options);
        if (this.host && !this.changed && this.hostKey === key) {
            this.host.greenFlag();
            return;
        }
        this.busy = true;
        this.cancelled = false;
        try {
            if (this.host) {
                // Carry what the old run left behind into the new one.
                const state = await this.host.requestState();
                if (state) this._sync(state);
                this._end();
            }
            if (this.changed || this.compiledKey !== key) {
                if (!await this._compile(options, key)) return;
            }
            if (!this.cancelled) await this._start(key);
        } finally {
            this.busy = false;
        }
    }

    async _compile (options, key) {
        const compiler = typeof window !== 'undefined' && window.scrustCompiler;
        if (!compiler) {
            this.status.error('scrust のコンパイラに接続できません。Electron 版 (gui/) から起動してください。');
            return false;
        }
        this.status.busy(COMPILE_HINT);
        // Edits made while compiling mark the result stale again.
        this.changed = false;
        this.compiledKey = null;
        try {
            const sb3 = await this.vm.saveProjectSb3('arraybuffer');
            const result = await compiler.compile(new Uint8Array(sb3), options);
            if (!result.ok) {
                this.changed = true;
                this.status.error(result.log || 'compile failed');
                return false;
            }
            this.compiledKey = key;
            if (result.warnings) this.status.notice(result.warnings);
            else this.status.clear();
            return true;
        } catch (e) {
            this.changed = true;
            this.status.error(String((e && e.stack) || e));
            return false;
        }
    }

    async _start (key) {
        const renderer = this.renderer;
        const ratio = renderer ? renderer.canvas.width / 480 : 1;
        const scale = Math.min(2, Math.max(1, Math.round(ratio * 2) / 2));
        const host = new ProgramHost({
            onFrame: (data, width, height) => {
                if (this.host === host && this.renderer) this.renderer.showProgramFrame(data, width, height);
            },
            onState: state => {
                if (this.host !== host) return;
                // The run finished: show its results in the editor.
                this._sync(state);
                if (this.changed) this._end();
            },
            onExit: message => {
                if (this.host !== host) return;
                this._end();
                this.status.error(`プログラムが終了しました (${message.signal || `code ${message.code}`})\n${message.log || ''}`);
            }
        });
        this.host = host;
        this.hostKey = key;
        try {
            const sharedFrames = !!(renderer && renderer.canReadProgramFrames);
            const {framesPath} = await host.start({scale, sharedFrames});
            if (framesPath && renderer) renderer.watchProgramFrames(framesPath);
        } catch (e) {
            this.host = null;
            this.hostKey = null;
            this.status.error(`プログラムを起動できませんでした: ${e}`);
            return;
        }
        document.body.classList.add('scrust-live');
        if (renderer) renderer.setProgramMode(true);
        this.vm.runtime.emit('PROJECT_START');
    }

    stop () {
        if (this.busy) this.cancelled = true;
        // The program reports its state once its threads are gone (onState).
        if (this.host) this.host.stop();
    }

    /** Leave program mode. */
    _end () {
        if (!this.host) return;
        this.host.dispose();
        this.host = null;
        this.hostKey = null;
        document.body.classList.remove('scrust-live');
        if (this.renderer) this.renderer.setProgramMode(false);
    }

    /** A new project replaces the old one: drop the program and the compiled executable. */
    reset () {
        this._end();
        this.compiledKey = null;
        this.changed = true;
        this.status.clear();
    }

    _onProjectChanged () {
        if (this.syncing) return;
        this.changed = true;
        // An idle program has already been synced; leave program mode so the
        // edit is visible. A running one keeps running until it stops.
        if (this.host && this.host.threadCount() === 0) this._end();
    }

    onScriptClicked () {
        this.status.flash('scrust: スクリプト単体の実行には対応していません。緑の旗でプロジェクト全体をコンパイルして実行します。');
    }

    // ---------------------------------------------------------------- input

    postIO (device, data) {
        const host = this.host;
        if (!host) return;
        const [w, h] = this.renderer ? this.renderer.getNativeSize() : [480, 360];
        if (device === 'mouse') {
            if (typeof data.x !== 'number' || !data.canvasWidth) return;
            const x = w * ((data.x / data.canvasWidth) - 0.5);
            const y = h * (0.5 - (data.y / data.canvasHeight));
            host.mouseMove(x, y);
            if (typeof data.isDown === 'boolean') host.mouseButton(x, y, data.isDown);
        } else if (device === 'keyboard') {
            // Escape and F5 are the standalone player's stop / green flag keys.
            if (typeof data.key !== 'string' || data.key === 'Escape' || data.key === 'F5') return;
            host.key(data.key, data.isDown);
        }
    }

    // ---------------------------------------------------------------- sync

    /** Copy a state snapshot of the program into the editor model. */
    _sync (state) {
        const rt = this.vm.runtime;
        this.syncing = true;
        try {
            const originals = rt.targets.filter(t => t.isOriginal);
            const find = s => originals.find(t => (s.stage ? t.isStage : !t.isStage && t.getName() === s.name));
            const layered = [];
            for (const s of state.targets) {
                const t = find(s);
                if (!t) continue;
                if (!s.stage) {
                    t.setXY(decodeValue(s.x), decodeValue(s.y), true);
                    t.setDirection(decodeValue(s.direction));
                    t.setSize(decodeValue(s.size));
                    t.setVisible(s.visible);
                    layered.push([s.layer, t]);
                }
                t.setCostume(s.costume);
                for (const [name, value] of s.vars) {
                    const v = t.lookupVariableByNameAndType(name, '', true);
                    if (v) v.value = decodeValue(value);
                }
                for (const [name, items] of s.lists) {
                    const v = t.lookupVariableByNameAndType(name, 'list', true);
                    if (v) {
                        v.value = items.map(decodeValue);
                        v._monitorUpToDate = false;
                    }
                }
            }
            layered.sort((a, b) => a[0] - b[0]).forEach(([, t]) => t.goToFront());
            for (const m of state.monitors) rt.requestUpdateMonitor({id: m.id, visible: m.visible});
            this._refreshMonitors(true);
            this.vm.emitTargetsUpdate(false);
        } finally {
            this.syncing = false;
        }
    }

    /**
     * Monitors normally get their values from scratch-vm threads, which never
     * run here; read them from the editor model instead.
     */
    _refreshMonitors (force) {
        if (this.host && !force) return;
        const rt = this.vm.runtime;
        if (!rt._monitorState || rt.targets.length === 0) return;
        const stage = rt.getTargetForStage();
        for (const m of rt._monitorState.values()) {
            const target = (m.targetId && rt.getTargetById(m.targetId)) || stage;
            if (!target) continue;
            const params = m.params || {};
            let value;
            switch (m.opcode) {
            case 'data_variable': {
                const v = target.lookupVariableByNameAndType(params.VARIABLE, '');
                if (!v) continue;
                value = v.value;
                break;
            }
            case 'data_listcontents': {
                const v = target.lookupVariableByNameAndType(params.LIST, 'list');
                if (!v) continue;
                if (sameList(v.value, m.value)) continue;
                value = v.value.slice();
                break;
            }
            case 'motion_xposition': value = target.x; break;
            case 'motion_yposition': value = target.y; break;
            case 'motion_direction': value = target.direction; break;
            case 'looks_size': value = Math.round(target.size); break;
            case 'sound_volume': value = target.volume; break;
            case 'looks_costumenumbername':
            case 'looks_backdropnumbername': {
                const t = m.opcode === 'looks_backdropnumbername' ? stage : target;
                const costume = t.getCostumes()[t.currentCostume];
                value = params.NUMBER_NAME === 'number' ? t.currentCostume + 1 : (costume ? costume.name : '');
                break;
            }
            default:
                continue;
            }
            if (m.value !== value) rt.requestUpdateMonitor({id: m.id, value});
        }
    }
}

class ScrustVM extends VirtualMachine {
    constructor () {
        super();
        this.scrust = new ScrustSession(this);
        disableExecution(this.runtime, this.scrust);
    }

    greenFlag () {
        this.scrust.greenFlag();
    }

    stopAll () {
        super.stopAll();
        this.scrust.stop();
    }

    postIOData (device, data) {
        this.scrust.postIO(device, data);
        super.postIOData(device, data);
    }

    loadProject (input) {
        this.scrust.reset();
        return super.loadProject(input);
    }

    clear () {
        this.scrust.reset();
        super.clear();
    }

    // TurboWarp's JS compiler is never used.
    setCompilerOptions (compilerOptions) {
        super.setCompilerOptions(Object.assign({}, compilerOptions, {enabled: false}));
    }
}

/**
 * Keep scratch-vm's runtime (the editor needs its targets, blocks, monitors
 * and extension metadata) but make sure it never executes a script.
 */
function disableExecution (runtime, session) {
    runtime.setCompilerOptions({enabled: false});
    runtime.startHats = () => [];
    runtime.addMonitorScript = () => {};
    runtime.toggleScript = () => session.onScriptClicked();
    runtime.sequencer.stepThreads = () => [];
    const pushThread = runtime._pushThread.bind(runtime);
    runtime._pushThread = (...args) => {
        const thread = pushThread(...args);
        runtime._stopThread(thread);
        return thread;
    };
    // The green flag in the GUI lights up while the compiled program runs.
    const emitRunStatus = runtime._emitProjectRunStatus.bind(runtime);
    runtime._emitProjectRunStatus = count => emitRunStatus(count + session.threadCount());
}

module.exports = ScrustVM;
