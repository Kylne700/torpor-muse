'use strict';
// Synthetic notes app: zero dependencies, in-memory data, one demo user.
const http = require('http');
const crypto = require('crypto');

const users = new Map([['alice', 'wonderland-demo-pass']]);
const sessions = new Map(); // session id -> user name
const notes = []; // { owner, text }

function sendJson(res, status, data, headers) {
  const text = JSON.stringify(data);
  res.writeHead(
    status,
    Object.assign({ 'content-type': 'application/json', 'content-length': Buffer.byteLength(text) }, headers),
  );
  res.end(text);
}

function sendPage(res, html) {
  res.writeHead(200, { 'content-type': 'text/html; charset=utf-8', 'content-length': Buffer.byteLength(html) });
  res.end(html);
}

function readJson(req, done) {
  let raw = '';
  req.on('data', (chunk) => {
    raw += chunk;
    if (raw.length > 10000) req.destroy();
  });
  req.on('end', () => {
    try {
      done(raw ? JSON.parse(raw) : {});
    } catch (err) {
      done(null);
    }
  });
}

function signedInUser(req) {
  const match = /(?:^|; )sid=([a-f0-9]+)/.exec(req.headers.cookie || '');
  return match ? sessions.get(match[1]) : undefined;
}

const server = http.createServer((req, res) => {
  const path = new URL(req.url, 'http://localhost').pathname;

  if (req.method === 'GET' && path === '/') {
    return sendPage(res, '<h1>Notes</h1><p>Small notes for small teams.</p><a href="/login">Log in</a>');
  }
  if (req.method === 'GET' && path === '/login') {
    return sendPage(res, '<h1>Log in</h1><form><input name="user"><input name="password" type="password"></form>');
  }
  // login:start
  if (req.method === 'POST' && path === '/api/login') {
    return readJson(req, (body) => {
      if (!body || users.get(body.user) !== body.password) {
        return sendJson(res, 401, { error: 'wrong user or password' });
      }
      const sid = crypto.randomBytes(8).toString('hex');
      sessions.set(sid, body.user);
      return sendJson(res, 200, { user: body.user }, { 'set-cookie': 'sid=' + sid + '; Path=/; HttpOnly' });
    });
  }
  // login:end
  if (req.method === 'GET' && path === '/api/me') {
    const user = signedInUser(req);
    return user ? sendJson(res, 200, { user }) : sendJson(res, 401, { error: 'not signed in' });
  }
  if (req.method === 'POST' && path === '/api/logout') {
    sessions.clear();
    return sendJson(res, 200, { ok: true }, { 'set-cookie': 'sid=; Path=/; Max-Age=0' });
  }
  if (path === '/api/notes') {
    const user = signedInUser(req);
    if (!user) return sendJson(res, 401, { error: 'not signed in' });
    if (req.method === 'GET') {
      return sendJson(res, 200, { notes: notes.filter((n) => n.owner === user).map((n) => n.text) });
    }
    if (req.method === 'POST') {
      return readJson(req, (body) => {
        if (!body || typeof body.text !== 'string' || body.text === '') {
          return sendJson(res, 400, { error: 'text is required' });
        }
        notes.push({ owner: user, text: body.text });
        return sendJson(res, 201, { saved: true });
      });
    }
  }
  return sendJson(res, 404, { error: 'not found' });
});

server.listen(Number(process.env.PORT) || 3000, '127.0.0.1');
