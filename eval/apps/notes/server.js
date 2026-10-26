'use strict';
// notes: a small notes service. Run it with: PORT=3000 node server.js
const http = require('http');
const auth = require('./lib/auth');
const store = require('./lib/store');

const PORT = Number(process.env.PORT) || 3000;

const PAGE = `<!doctype html>
<title>notes</title>
<h1>notes</h1>
<p>Log in with POST /login, then use /notes.</p>
`;

function send(res, status, body, headers = {}) {
  const isText = typeof body === 'string';
  res.writeHead(status, { 'Content-Type': isText ? 'text/html; charset=utf-8' : 'application/json', ...headers });
  res.end(isText ? body : JSON.stringify(body));
}

function readJson(req) {
  return new Promise((resolve, reject) => {
    let raw = '';
    req.on('data', (chunk) => (raw += chunk));
    req.on('end', () => {
      try {
        resolve(raw ? JSON.parse(raw) : {});
      } catch (err) {
        reject(Object.assign(new Error('invalid JSON'), { status: 400 }));
      }
    });
  });
}

async function handle(req, res) {
  const url = new URL(req.url, 'http://localhost');
  const path = url.pathname;
  const method = req.method;

  if (method === 'GET' && path === '/') return send(res, 200, PAGE);

  if (method === 'POST' && path === '/register') {
    const { username, password } = await readJson(req);
    if (!auth.validUsername(username) || !auth.validPassword(password)) {
      return send(res, 400, { error: 'invalid username or password' });
    }
    if (store.findUser(username)) return send(res, 409, { error: 'username taken' });
    const user = store.addUser(username, auth.hashPassword(password));
    return send(res, 201, { id: user.id, username: user.username });
  }

  if (method === 'POST' && path === '/login') {
    const { username, password } = await readJson(req);
    const user = store.findUser(username);
    if (!user || !auth.verifyPassword(password, user.password)) {
      return send(res, 401, { error: 'wrong username or password' });
    }
    const sid = auth.createSession(user.id);
    return send(res, 200, { username: user.username }, { 'Set-Cookie': `sid=${sid}; HttpOnly; Path=/` });
  }

  if (method === 'POST' && path === '/logout') {
    return send(res, 200, { ok: true }, { 'Set-Cookie': 'sid=; Max-Age=0; Path=/' });
  }

  const user = auth.currentUser(req);
  if (!user) return send(res, 401, { error: 'login required' });

  if (method === 'GET' && path === '/me') return send(res, 200, { id: user.id, username: user.username });

  if (path === '/notes' && method === 'GET') {
    let rows = store.notesOf(user.id);
    return send(res, 200, rows);
  }

  if (path === '/notes' && method === 'POST') {
    const { title, body } = await readJson(req);
    if (typeof title !== 'string' || !title.trim()) return send(res, 400, { error: 'title is required' });
    return send(res, 201, store.addNote(user.id, title.trim(), typeof body === 'string' ? body : ''));
  }

  const m = /^\/notes\/(\d+)$/.exec(path);
  if (m) {
    const note = store.getNote(Number(m[1]));
    if (!note || note.userId !== user.id) return send(res, 404, { error: 'not found' });
    if (method === 'GET') return send(res, 200, note);
    if (method === 'DELETE') {
      store.deleteNote(note.id);
      res.writeHead(204);
      return res.end();
    }
  }

  return send(res, 404, { error: 'not found' });
}

http
  .createServer((req, res) => {
    handle(req, res).catch((err) => send(res, err.status || 500, { error: err.status ? err.message : 'server error' }));
  })
  .listen(PORT, '127.0.0.1', () => console.log(`notes listening on ${PORT}`));
