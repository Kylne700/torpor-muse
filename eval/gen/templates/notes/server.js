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
//#if S4
  if (method === 'GET' && path === '/health') return send(res, 200, { status: 'ok' });
//#endif

  if (method === 'POST' && path === '/register') {
    const { username, password } = await readJson(req);
    if (!auth.validUsername(username) || !auth.validPassword(password)) {
      return send(res, 400, { error: 'invalid username or password' });
    }
//#if I_dup
    if (store.findUser(String(username).toUpperCase())) return send(res, 409, { error: 'username taken' });
//#else
    if (store.findUser(username)) return send(res, 409, { error: 'username taken' });
//#endif
    const user = store.addUser(username, auth.hashPassword(password));
    return send(res, 201, { id: user.id, username: user.username });
  }

  if (method === 'POST' && path === '/login') {
    const { username, password } = await readJson(req);
//#if R_rl=ref
    const key = String(username).toLowerCase();
//#endif
//#if R_rl=slip
    const key = req.socket.remoteAddress;
//#endif
//#if R_rl
    if (auth.isLocked(key)) {
      return send(res, 429, { error: 'too many failed logins, try again later' }, { 'Retry-After': '60' });
    }
//#endif
//#if E_case
    const user = store.findUser(String(username).toLowerCase());
//#else
    const user = store.findUser(username);
//#endif
    if (!user || !auth.verifyPassword(password, user.password)) {
//#if R_rl
      auth.recordFailure(key);
//#endif
      return send(res, 401, { error: 'wrong username or password' });
    }
//#if R_rl
    auth.clearFailures(key);
//#endif
    const sid = auth.createSession(user.id);
//#if R_cookie
    return send(res, 200, { username: user.username }, { 'Set-Cookie': `notes_session=${sid}; HttpOnly; Path=/` });
//#else
    return send(res, 200, { username: user.username }, { 'Set-Cookie': `sid=${sid}; HttpOnly; Path=/` });
//#endif
  }

  if (method === 'POST' && path === '/logout') {
//#if E_logout
    auth.destroySession(auth.sessionId(req));
//#endif
//#if R_cookie
    return send(res, 200, { ok: true }, { 'Set-Cookie': 'notes_session=; Max-Age=0; Path=/' });
//#else
    return send(res, 200, { ok: true }, { 'Set-Cookie': 'sid=; Max-Age=0; Path=/' });
//#endif
  }

  const user = auth.currentUser(req);
  if (!user) return send(res, 401, { error: 'login required' });

  if (method === 'GET' && path === '/me') return send(res, 200, { id: user.id, username: user.username });
//#if S5
  if (method === 'GET' && path === '/stats') return send(res, 200, { notes: store.notesOf(user.id).length });
//#endif
//#if S6
  if (method === 'GET' && path === '/export') {
    const rows = store.notesOf(user.id).map((n) => ({ id: n.id, title: n.title, body: n.body }));
    return send(res, 200, rows, { 'Content-Disposition': 'attachment; filename="notes.json"' });
  }
//#endif

  if (path === '/notes' && method === 'GET') {
    let rows = store.notesOf(user.id);
//#if N_search
    const q = (url.searchParams.get('q') || '').toLowerCase();
    if (q) rows = rows.filter((n) => (n.title + ' ' + n.body).toLowerCase().includes(q));
//#endif
//#if N_sort
    if (url.searchParams.get('sort') === 'title') {
      rows = [...rows].sort((a, b) => a.title.toLowerCase().localeCompare(b.title.toLowerCase()));
    }
//#endif
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
//#if E_edit
    if (method === 'PUT') {
      const { title, body } = await readJson(req);
      const fields = {};
      if (typeof title === 'string' && title.trim()) fields.title = title.trim();
      if (typeof body === 'string') fields.body = body;
      return send(res, 200, store.updateNote(note, fields));
    }
//#endif
  }

  return send(res, 404, { error: 'not found' });
}

http
  .createServer((req, res) => {
    handle(req, res).catch((err) => send(res, err.status || 500, { error: err.status ? err.message : 'server error' }));
  })
  .listen(PORT, '127.0.0.1', () => console.log(`notes listening on ${PORT}`));
