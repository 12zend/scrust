// Static server for ../out/pixeltest that also accepts the reference frame:
//   POST /reference.png  -> conformance/pixel/reference.png
const http = require('http');
const fs = require('fs');
const path = require('path');
const dir = path.join(__dirname, '..', 'out', 'pixeltest');
const types = {'.html': 'text/html', '.png': 'image/png', '.sb3': 'application/octet-stream', '.js': 'text/javascript'};
http.createServer((req, res) => {
    const url = req.url.split('?')[0];
    if (req.method === 'POST' && url === '/reference.png') {
        const chunks = [];
        req.on('data', c => chunks.push(c));
        req.on('end', () => { fs.writeFileSync(path.join(__dirname, 'reference.png'), Buffer.concat(chunks)); res.end('saved'); });
        return;
    }
    const file = path.join(dir, url === '/' ? 'index.html' : path.basename(url));
    fs.readFile(file, (err, data) => {
        if (err) { res.statusCode = 404; res.end('not found'); return; }
        res.setHeader('Content-Type', types[path.extname(file)] || 'application/octet-stream');
        res.setHeader('Cache-Control', 'no-store');
        res.end(data);
    });
}).listen(Number(process.argv[2] || 8767), () => console.log('serving', dir));
