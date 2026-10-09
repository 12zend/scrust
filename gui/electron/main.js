// Electron main process: serves the built GUI, compiles projects with the
// scrust CLI when the GUI's green flag asks for it, and runs the compiled
// native program as a child process whose frames are forwarded to the page.

const {app, BrowserWindow, ipcMain, net, protocol, shell} = require('electron');
const {spawn} = require('child_process');
const fs = require('fs');
const os = require('os');
const path = require('path');
const {pathToFileURL} = require('url');

const REPO = path.resolve(__dirname, '..', '..');
const BUILD = path.resolve(__dirname, '..', 'build');
const EXE = process.platform === 'win32' ? '.exe' : '';
const SCRUST = process.env.SCRUST_BIN || path.join(REPO, 'target', 'release', `scrust${EXE}`);
// The input's file stem names the generated crate (target/scrust-gen/gui_project),
// so every compile reuses the same crate and cargo only rebuilds what changed.
const STEM = 'gui_project';
const SCHEME = 'scrust-gui';

// cargo is usually not on PATH when the app is started from the Finder.
process.env.PATH = [path.join(os.homedir(), '.cargo', 'bin'), '/opt/homebrew/bin', '/usr/local/bin', process.env.PATH]
    .join(path.delimiter);

protocol.registerSchemesAsPrivileged([{
    scheme: SCHEME,
    privileges: {standard: true, secure: true, supportFetchAPI: true, corsEnabled: true, stream: true}
}]);

const workDir = () => path.join(app.getPath('temp'), 'scrust-gui');
const programPath = () => path.join(workDir(), 'out', `${STEM}${EXE}`);

const run = (cmd, args, options) => new Promise(resolve => {
    const child = spawn(cmd, args, Object.assign({stdio: ['ignore', 'pipe', 'pipe']}, options));
    let stdout = '';
    let stderr = '';
    child.stdout.on('data', d => {
        stdout += d;
    });
    child.stderr.on('data', d => {
        stderr += d;
    });
    child.on('error', e => resolve({code: -1, stdout, stderr: `${stderr}${e.message}`}));
    child.on('close', code => resolve({code, stdout, stderr}));
});

// ------------------------------------------------------------------ program

/** The running program: {child, sender, stderr}. */
let program = null;

let runs = 0;

/** Remove a run's shared frame file (the page may still have it mapped; that is fine). */
const removeFrames = file => {
    if (file) fs.rm(file, {force: true}, () => {});
};

function stopProgram () {
    if (!program) return;
    const {child} = program;
    program = null;
    child.stdin.end();
    // The program exits when its input closes; make sure it does.
    setTimeout(() => {
        if (child.exitCode === null) child.kill();
    }, 1000);
}

function startProgram (sender, options) {
    stopProgram();
    const exe = programPath();
    if (!fs.existsSync(exe)) throw new Error('no compiled program');
    const args = ['--embed', '--scale', String(options.scale || 1)];
    // Frames go through a shared memory file that the page reads with the
    // renderer addon (scrust_render::frames), when the page can; otherwise
    // they are piped through this process.
    const framesPath = options.sharedFrames ? path.join(workDir(), `frames-${process.pid}-${++runs}.bin`) : null;
    if (framesPath) args.push('--frames-shm', framesPath);
    const child = spawn(exe, args, {
        cwd: workDir(),
        stdio: ['pipe', 'pipe', 'pipe']
    });
    // frameBusy: a frame was sent to the page and not drawn yet (it answers
    // 'scrust:frame-ack'); newer frames wait in nextFrame, replacing each
    // other, so a slow page shows the latest picture instead of a backlog.
    const me = {child, sender, stderr: '', frameBusy: false, nextFrame: null};
    program = me;
    const post = message => {
        if (!sender.isDestroyed()) sender.send('scrust:program', message);
    };
    me.postFrame = frame => {
        if (me.frameBusy) {
            me.nextFrame = frame;
            return;
        }
        me.frameBusy = true;
        post(frame);
    };
    // Messages are [tag u8][length u32 LE][payload]; see crates/scrust-player/src/embed.rs.
    // Chunks are kept in a list and joined once a whole message is there
    // (joining on every 64 KiB chunk copies a large frame dozens of times).
    let chunks = [];
    let buffered = 0;
    let need = 5;
    child.stdout.on('data', chunk => {
        chunks.push(chunk);
        buffered += chunk.length;
        if (buffered < need) return;
        let pending = chunks.length === 1 ? chunks[0] : Buffer.concat(chunks, buffered);
        while (pending.length >= 5) {
            const length = pending.readUInt32LE(1);
            if (pending.length < 5 + length) break;
            const tag = String.fromCharCode(pending[0]);
            const payload = pending.subarray(5, 5 + length);
            if (tag === 'F') {
                me.postFrame({type: 'frame', width: payload.readUInt32LE(0), height: payload.readUInt32LE(4), data: payload.subarray(8)});
            } else if (tag === 'T') {
                post({type: 'threads', count: payload.readUInt32LE(0)});
            } else if (tag === 'S') {
                post({type: 'state', json: payload.toString('utf8')});
            }
            pending = pending.subarray(5 + length);
        }
        chunks = pending.length ? [pending] : [];
        buffered = pending.length;
        need = pending.length >= 5 ? 5 + pending.readUInt32LE(1) : 5;
    });
    child.stderr.on('data', d => {
        me.stderr = (me.stderr + d).slice(-4000);
    });
    child.stdin.on('error', () => {});
    child.on('exit', (code, signal) => {
        removeFrames(framesPath);
        // A program replaced by a newer one is expected to go away.
        if (program === me) {
            program = null;
            post({type: 'exit', code, signal, log: me.stderr});
        }
    });
    return {framesPath};
}

ipcMain.handle('scrust:start', (event, options) => startProgram(event.sender, options || {}));
ipcMain.on('scrust:input', (event, line) => {
    if (program && program.child.stdin.writable) program.child.stdin.write(`${line}\n`);
});
ipcMain.on('scrust:kill', () => stopProgram());
ipcMain.on('scrust:frame-ack', () => {
    if (!program) return;
    program.frameBusy = false;
    const frame = program.nextFrame;
    program.nextFrame = null;
    if (frame) program.postFrame(frame);
});

// ------------------------------------------------------------------ compiler

async function compile (sb3, options) {
    if (!fs.existsSync(SCRUST)) {
        return {ok: false, log: `scrust CLI not found at ${SCRUST}\nBuild it with: cargo build --release -p scrust-cli`};
    }
    // The executable is about to be replaced.
    stopProgram();
    const work = workDir();
    const out = path.join(work, 'out');
    fs.mkdirSync(out, {recursive: true});
    const input = path.join(work, `${STEM}.sb3`);
    fs.writeFileSync(input, Buffer.from(sb3));

    // `check` exits with 2 and lists the blocks scrust cannot compile; the
    // build below turns them into no-ops and the GUI shows the list.
    const check = await run(SCRUST, ['check', input], {cwd: REPO});
    let warnings = '';
    if (check.code === 2) {
        warnings = check.stdout.split('\n').filter(l => l.startsWith('  ')).join('\n');
    } else if (check.code !== 0) {
        return {ok: false, log: `${check.stderr}${check.stdout}`.trim()};
    }

    const args = ['build', input, '-o', out, '--allow-unsupported', '--fps', String(options.fps || 30)];
    if (options.turbo) args.push('--turbo');
    if (process.env.SCRUST_GUI_DEBUG) args.push('--debug');
    const started = Date.now();
    const build = await run(SCRUST, args, {cwd: REPO});
    if (build.code !== 0) return {ok: false, log: `${build.stderr}${build.stdout}`.trim()};
    console.log(`scrust: compiled in ${Date.now() - started} ms`);
    return {ok: true, warnings, log: ''};
}

// One compile at a time: they share the generated crate and cargo's target dir.
let queue = Promise.resolve();
ipcMain.handle('scrust:compile', (event, sb3, options) => {
    const job = queue.then(() => compile(sb3, options || {}));
    queue = job.catch(() => {});
    return job;
});

// ------------------------------------------------------------------ window

function serveBuild (request) {
    const {pathname} = new URL(request.url);
    const rel = decodeURIComponent(pathname).replace(/^\/+/, '') || 'editor.html';
    const file = path.resolve(BUILD, rel);
    if (!file.startsWith(BUILD + path.sep) || !fs.existsSync(file) || fs.statSync(file).isDirectory()) {
        return new Response('not found', {status: 404});
    }
    return net.fetch(pathToFileURL(file).toString());
}

function createWindow () {
    const win = new BrowserWindow({
        width: 1280,
        height: 820,
        title: 'scrust',
        webPreferences: {
            preload: path.join(__dirname, 'preload.js'),
            contextIsolation: true,
            nodeIntegration: false,
            // The preload loads the renderer addon (a native Node-API module).
            sandbox: false
        }
    });
    win.webContents.setWindowOpenHandler(({url}) => {
        if (url.startsWith(`${SCHEME}:`)) return {action: 'allow'};
        shell.openExternal(url);
        return {action: 'deny'};
    });
    win.on('closed', stopProgram);
    win.loadURL(`${SCHEME}://app/editor.html`);
}

app.whenReady().then(() => {
    if (!fs.existsSync(path.join(BUILD, 'editor.html'))) {
        console.error(`GUI build not found in ${BUILD}; run \`npm run build\` in gui/ first.`);
    }
    protocol.handle(SCHEME, serveBuild);
    createWindow();
    app.on('activate', () => {
        if (BrowserWindow.getAllWindows().length === 0) createWindow();
    });
});

app.on('window-all-closed', () => app.quit());
app.on('will-quit', stopProgram);
