#!/usr/bin/env node
// Static file server for trying the browser build (gui/build-web) locally.
// The build needs nothing from a server - projects run in the page - so any
// static host works the same (Cloudflare Pages, GitHub Pages, ...).
//
//   node gui/web/server.js                  http://127.0.0.1:8080
//   PORT=9000 HOST=0.0.0.0 node gui/web/server.js
//
// Environment: PORT, HOST (default 127.0.0.1), SCRUST_STATIC (directory to
// serve, default gui/build-web).

const http = require('http');
const fs = require('fs');
const path = require('path');

const ROOT = process.env.SCRUST_STATIC ? path.resolve(process.env.SCRUST_STATIC) : path.resolve(__dirname, '..', 'build-web');
const PORT = Number(process.env.PORT || 8080);
const HOST = process.env.HOST || '127.0.0.1';

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

const server = http.createServer((req, res) => {
    if (req.method !== 'GET' && req.method !== 'HEAD') {
        res.writeHead(405);
        res.end();
        return;
    }
    const {pathname} = new URL(req.url, 'http://localhost');
    const rel = decodeURIComponent(pathname).replace(/^\/+/, '') || 'index.html';
    const file = path.resolve(ROOT, rel);
    if (!file.startsWith(ROOT + path.sep) || !fs.existsSync(file) || fs.statSync(file).isDirectory()) {
        res.writeHead(404, {'Content-Type': 'text/plain; charset=utf-8'});
        res.end('not found');
        return;
    }
    res.writeHead(200, {'Content-Type': MIME[path.extname(file).toLowerCase()] || 'application/octet-stream'});
    if (req.method === 'HEAD') {
        res.end();
        return;
    }
    fs.createReadStream(file).pipe(res);
});

server.listen(PORT, HOST, () => {
    if (!fs.existsSync(path.join(ROOT, 'index.html'))) {
        console.error(`GUI build not found in ${ROOT}; run \`npm run build:web\` in gui/ first.`);
    }
    console.log(`scrust web GUI: http://${HOST === '0.0.0.0' ? 'localhost' : HOST}:${PORT}/`);
});
