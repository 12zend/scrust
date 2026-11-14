// Server for the TurboWarp reference page.
//   node serve.js <port> <sb3 dir>
// GET /               index.html (see entry.js for the query parameters)
// GET /twref.js       the bundle (conformance/out/twref)
// GET /p/<name>       a project from <sb3 dir>; GET /list  their names
// GET /r/<name>       a result file
// POST /save?name=N[&append=1]   writes conformance/out/twref/results/N
const http = require('http');
const fs = require('fs');
const path = require('path');
const port = Number(process.argv[2] || 8768);
const projects = process.argv[3] || '/Users/kobajin/Downloads/sb3bench';
const out = path.join(__dirname, '..', 'out', 'twref');
const results = path.join(out, 'results');
fs.mkdirSync(results, {recursive: true});
const page = '<!doctype html><meta charset="utf-8"><title>TurboWarp reference</title><canvas id="c" width="480" height="360"></canvas><pre id="log"></pre><script src="twref.js"></script>';
http.createServer((req, res) => {
    const u = new URL(req.url, 'http://x');
    res.setHeader('Cache-Control', 'no-store');
    if (req.method === 'POST' && u.pathname === '/save') {
        const name = path.basename(u.searchParams.get('name') || 'out');
        const chunks = [];
        req.on('data', c => chunks.push(c));
        req.on('end', () => {
            const f = path.join(results, name);
            if (u.searchParams.get('append')) fs.appendFileSync(f, Buffer.concat(chunks)); else fs.writeFileSync(f, Buffer.concat(chunks));
            res.end('ok');
        });
        return;
    }
    let file;
    if (u.pathname === '/' || u.pathname === '/index.html') { res.setHeader('Content-Type', 'text/html'); res.end(page); return; }
    if (u.pathname === '/list') {
        res.setHeader('Content-Type', 'application/json');
        res.end(JSON.stringify(fs.readdirSync(projects).filter(f => f.endsWith('.sb3')).sort()));
        return;
    }
    if (u.pathname === '/twref.js') file = path.join(out, 'twref.js');
    else if (u.pathname.startsWith('/r/')) file = path.join(results, path.basename(decodeURIComponent(u.pathname.slice(3))));
    else if (u.pathname.startsWith('/p/')) file = path.join(projects, path.basename(decodeURIComponent(u.pathname.slice(3))));
    if (!file) { res.statusCode = 404; res.end(); return; }
    fs.readFile(file, (err, data) => {
        if (err) { res.statusCode = 404; res.end('not found'); return; }
        res.setHeader('Content-Type', file.endsWith('.js') ? 'text/javascript' : 'application/octet-stream');
        res.end(data);
    });
}).listen(port, () => console.log(`twref on http://localhost:${port}/ serving ${projects}`));
