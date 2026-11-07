// Servidor estático para ver a interface no navegador, sem o Tauri.
// Serve ../src e injeta dev/mock-tauri.js antes do main.js.
//   npm run preview   ->  http://localhost:5174
import { createServer } from 'node:http';
import { readFile } from 'node:fs/promises';
import { extname, join, normalize } from 'node:path';
import { fileURLToPath } from 'node:url';

const here = fileURLToPath(new URL('.', import.meta.url));
const root = join(here, '..', 'src');
const port = Number(process.env.PORT) || 5174;
const types = {
  '.html': 'text/html; charset=utf-8',
  '.js': 'text/javascript; charset=utf-8',
  '.css': 'text/css; charset=utf-8',
  '.woff2': 'font/woff2',
  '.png': 'image/png',
  '.svg': 'image/svg+xml',
};

createServer(async (req, res) => {
  const url = decodeURIComponent(new URL(req.url, 'http://x').pathname);
  try {
    if (url === '/__mock.js') {
      res.writeHead(200, { 'content-type': types['.js'] });
      return res.end(await readFile(join(here, 'mock-tauri.js')));
    }
    const path = normalize(join(root, url === '/' ? 'index.html' : url));
    if (!path.startsWith(root)) throw new Error('fora da pasta');
    let body = await readFile(path);
    if (path.endsWith('index.html')) {
      body = body.toString().replace('<script src="main.js">', '<script src="/__mock.js"></script>\n  <script src="main.js">');
    }
    res.writeHead(200, { 'content-type': types[extname(path)] || 'application/octet-stream' });
    res.end(body);
  } catch {
    res.writeHead(404);
    res.end('não encontrado');
  }
}).listen(port, () => console.log(`Halo Studio (mock) em http://localhost:${port}`));
