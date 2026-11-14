#!/usr/bin/env node
// Compile server for the browser build of the GUI (gui/web): the green flag
// posts the project here, scrust compiles it to Rust and cargo builds it for
// wasm32-unknown-unknown; the page runs the returned module itself. It also
// serves the built page (gui/build-web), so one process is enough locally.
//
//   node gui/web/server.js                  http://127.0.0.1:8080
//   PORT=9000 HOST=0.0.0.0 node gui/web/server.js
//
// API:
//   POST /api/compile?fps=30&turbo=0   body: the project as .sb3
//     200 application/wasm             the program; unsupported blocks (which
//                                      become no-ops) in X-Scrust-Warnings,
//                                      URI-encoded
//     422 text/plain                   the compiler's log
//
// Environment: PORT, HOST (default 127.0.0.1: compiling runs cargo, so it is
// not exposed unless asked), SCRUST_BIN (CLI path), SCRUST_CORS_ORIGIN
// (Access-Control-Allow-Origin for a page served elsewhere, e.g. Cloudflare
// Pages; default *), SCRUST_GUI_DEBUG=1 (unoptimised builds), SCRUST_STATIC
// (directory to serve, default gui/build-web; empty: API only).

const http = require('http');
const {spawn} = require('child_process');
const fs = require('fs');
const os = require('os');
const path = require('path');

const REPO = path.resolve(__dirname, '..', '..');
const EXE = process.platform === 'win32' ? '.exe' : '';
const SCRUST = process.env.SCRUST_BIN || path.join(REPO, 'target', 'release', `scrust${EXE}`);
const STATIC = process.env.SCRUST_STATIC === undefined ? path.join(REPO, 'gui', 'build-web') :
    (process.env.SCRUST_STATIC && path.resolve(process.env.SCRUST_STATIC));
const PORT = Number(process.env.PORT || 8080);
const HOST = process.env.HOST || '127.0.0.1';
const CORS_ORIGIN = process.env.SCRUST_CORS_ORIGIN || '*';
const MAX_BODY = 200 * 1024 * 1024;
// The input's file stem names the generated crate (target/scrust-gen/gui_project_web),
// so every compile reuses the same crate and cargo only rebuilds what changed.
const STEM = 'gui_project';
const WORK = path.join(os.tmpdir(), 'scrust-gui-web');

// cargo and rustup are usually in ~/.cargo/bin.
process.env.PATH = [path.join(os.homedir(), '.cargo', 'bin'), process.env.PATH].join(path.delimiter);

const MIME = {
    '.html': 'text/html; charset=utf-8',
    '.js': 'text/javascript; charset=utf-8',
    '.css': 'text/css; charset=utf-8',
    '.json': 'application/json',
    '.webmanifest': 'application/manifest+json',
    '.wasm': 'application/wasm',
    '.svg': 'image/svg+xml',
    '.png': 'image/png',
    '.jpg': 'image/jpeg',
    '.gif': 'image/gif',
    '.ico': 'image/x-icon',
    '.woff': 'font/woff',
    '.woff2': 'font/woff2',
    '.ttf': 'font/ttf',
    '.wav': 'audio/wav',
    '.mp3': 'audio/mpeg',
    '.txt': 'text/plain; charset=utf-8'
};

const run = (cmd, args) => new Promise(resolve => {
    const child = spawn(cmd, args, {cwd: REPO, stdio: ['ignore', 'pipe', 'pipe']});
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

/** @returns {Promise<{ok: true, wasm: Buffer, warnings: string} | {ok: false, log: string}>} */
async function compile (sb3, options) {
    if (!fs.existsSync(SCRUST)) {
        return {ok: false, log: `scrust CLI not found at ${SCRUST}\nBuild it with: cargo build --release -p scrust-cli`};
    }
    const out = path.join(WORK, 'out');
    fs.mkdirSync(out, {recursive: true});
    const input = path.join(WORK, `${STEM}.sb3`);
    fs.writeFileSync(input, sb3);

    // `check` exits with 2 and lists the blocks scrust cannot compile; the
    // build below turns them into no-ops and the GUI shows the list.
    const check = await run(SCRUST, ['check', input]);
    let warnings = '';
    if (check.code === 2) {
        warnings = check.stdout.split('\n').filter(l => l.startsWith('  ')).join('\n');
    } else if (check.code !== 0) {
        return {ok: false, log: `${check.stderr}${check.stdout}`.trim()};
    }

    const args = ['build', input, '-o', out, '--target', 'wasm', '--allow-unsupported', '--fps', String(options.fps)];
    if (options.turbo) args.push('--turbo');
    if (process.env.SCRUST_GUI_DEBUG) args.push('--debug');
    const started = Date.now();
    const build = await run(SCRUST, args);
    if (build.code !== 0) return {ok: false, log: `${build.stderr}${build.stdout}`.trim()};
    console.log(`scrust: compiled to wasm in ${Date.now() - started} ms`);
    return {ok: true, wasm: fs.readFileSync(path.join(out, `${STEM}_web.wasm`)), warnings};
}

// One compile at a time: they share the generated crate and cargo's target dir.
let queue = Promise.resolve();
const enqueue = job => {
    const result = queue.then(job);
    queue = result.catch(() => {});
    return result;
};

const readBody = req => new Promise((resolve, reject) => {
    const chunks = [];
    let size = 0;
    req.on('data', chunk => {
        size += chunk.length;
        if (size > MAX_BODY) {
            reject(new Error('project too large'));
            req.destroy();
            return;
        }
        chunks.push(chunk);
    });
    req.on('end', () => resolve(Buffer.concat(chunks)));
    req.on('error', reject);
});

const cors = res => {
    res.setHeader('Access-Control-Allow-Origin', CORS_ORIGIN);
    res.setHeader('Access-Control-Allow-Methods', 'POST, OPTIONS');
    res.setHeader('Access-Control-Allow-Headers', 'Content-Type');
    res.setHeader('Access-Control-Expose-Headers', 'X-Scrust-Warnings');
};

const text = (res, status, body) => {
    res.writeHead(status, {'Content-Type': 'text/plain; charset=utf-8'});
    res.end(body);
};

async function handleCompile (req, res, url) {
    cors(res);
    if (req.method === 'OPTIONS') {
        res.writeHead(204);
        res.end();
        return;
    }
    if (req.method !== 'POST') return text(res, 405, 'POST an .sb3');
    let body;
    try {
        body = await readBody(req);
    } catch (e) {
        return text(res, 413, e.message);
    }
    const fps = Number(url.searchParams.get('fps'));
    const options = {
        fps: Number.isFinite(fps) && fps > 0 && fps <= 250 ? fps : 30,
        turbo: url.searchParams.get('turbo') === '1'
    };
    const result = await enqueue(() => compile(body, options)).catch(e => ({ok: false, log: String((e && e.stack) || e)}));
    if (!result.ok) return text(res, 422, result.log || 'compile failed');
    res.writeHead(200, {
        'Content-Type': 'application/wasm',
        'Content-Length': result.wasm.length,
        'Cache-Control': 'no-store',
        'X-Scrust-Warnings': encodeURIComponent(result.warnings)
    });
    res.end(result.wasm);
}

function serveStatic (req, res, url) {
    if (!STATIC) return text(res, 404, 'not found');
    if (req.method !== 'GET' && req.method !== 'HEAD') return text(res, 405, 'method not allowed');
    const rel = decodeURIComponent(url.pathname).replace(/^\/+/, '') || 'index.html';
    const file = path.resolve(STATIC, rel);
    if (!file.startsWith(STATIC + path.sep) || !fs.existsSync(file) || fs.statSync(file).isDirectory()) {
        return text(res, 404, 'not found');
    }
    res.writeHead(200, {'Content-Type': MIME[path.extname(file).toLowerCase()] || 'application/octet-stream'});
    if (req.method === 'HEAD') return res.end();
    fs.createReadStream(file).pipe(res);
}

const server = http.createServer((req, res) => {
    const url = new URL(req.url, 'http://localhost');
    if (url.pathname === '/api/compile') {
        handleCompile(req, res, url).catch(e => text(res, 500, String(e)));
    } else {
        serveStatic(req, res, url);
    }
});

server.listen(PORT, HOST, () => {
    if (STATIC && !fs.existsSync(path.join(STATIC, 'index.html'))) {
        console.error(`GUI build not found in ${STATIC}; run \`npm run build:web\` in gui/ first.`);
    }
    console.log(`scrust web GUI: http://${HOST === '0.0.0.0' ? 'localhost' : HOST}:${PORT}/`);
});
