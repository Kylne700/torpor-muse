'use strict';
// todo: a small todo API with bearer tokens. Run it with: PORT=3000 node server.js
const http = require('http');
const auth = require('./lib/auth');
const db = require('./lib/db');

const PORT = Number(process.env.PORT) || 3000;

const PAGE = `<!doctype html>
<title>todo</title>
<h1>todo</h1>
<p>Get a token with POST /token, then call /todos with an Authorization: Bearer header.</p>
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
        const body = raw ? JSON.parse(raw) : {};
        resolve(body && typeof body === 'object' ? body : {});
      } catch (err) {
        reject(Object.assign(new Error('invalid JSON'), { status: 400 }));
      }
    });
  });
}

function present(todo) {
  return {
    id: todo.id,
    title: todo.title,
    done: todo.done,
  };
}

function home({ res }) {
  send(res, 200, PAGE);
}

async function signup({ req, res }) {
  const { username, password } = await readJson(req);
  if (!auth.validUsername(username) || !auth.validPassword(password)) {
    return send(res, 400, { error: 'invalid username or password' });
  }
  if (db.findUser(username)) return send(res, 409, { error: 'username taken' });
  const user = db.addUser(username, auth.hashPassword(password));
  send(res, 201, { id: user.id, username: user.username });
}

async function login({ req, res }) {
  const { username, password } = await readJson(req);
  const user = db.findUser(username);
  if (!user || !auth.verifyPassword(password, user.password)) {
    return send(res, 401, { error: 'wrong username or password' });
  }
  send(res, 200, { token: auth.issueToken(user.id) });
}

function listTodos({ res, url, user }) {
  let rows = db.todosOf(user.id);
  send(res, 200, rows.map(present));
}

async function createTodo({ req, res, user }) {
  const body = await readJson(req);
  if (typeof body.title !== 'string' || !body.title.trim()) return send(res, 400, { error: 'title is required' });
  const todo = db.addTodo(user.id, {
    title: body.title.trim(),
    done: body.done === true,
  });
  send(res, 201, present(todo));
}

function getTodo({ res, params, user }) {
  const todo = db.getTodo(user.id, Number(params[0]));
  if (!todo) return send(res, 404, { error: 'not found' });
  send(res, 200, present(todo));
}

async function patchTodo({ req, res, params, user }) {
  const todo = db.getTodo(user.id, Number(params[0]));
  if (!todo) return send(res, 404, { error: 'not found' });
  const body = await readJson(req);
  if (typeof body.title === 'string' && body.title.trim()) todo.title = body.title.trim();
  send(res, 200, present(todo));
}

function deleteTodo({ res, params, user }) {
  const todo = db.getTodo(user.id, Number(params[0]));
  if (!todo) return send(res, 404, { error: 'not found' });
  db.removeTodo(todo);
  res.writeHead(204);
  res.end();
}

async function dispatch(req, res) {
  const url = new URL(req.url, 'http://localhost');
  const { pathname } = url;
  const method = req.method;
  const ctx = { req, res, url, params: [], user: null };

  if (method === 'GET' && pathname === '/') return home(ctx);
  if (method === 'POST' && pathname === '/signup') return signup(ctx);
  if (method === 'POST' && pathname === '/token') return login(ctx);

  ctx.user = auth.userFor(req);
  if (!ctx.user) return send(res, 401, { error: 'login required' });

  if (pathname === '/todos' && method === 'GET') return listTodos(ctx);
  if (pathname === '/todos' && method === 'POST') return createTodo(ctx);
  const one = /^\/todos\/(\d+)$/.exec(pathname);
  if (one) {
    ctx.params = [one[1]];
    if (method === 'GET') return getTodo(ctx);
    if (method === 'PATCH') return patchTodo(ctx);
    if (method === 'DELETE') return deleteTodo(ctx);
  }
  send(res, 404, { error: 'not found' });
}

http
  .createServer((req, res) => {
    dispatch(req, res).catch((err) => {
      if (!err.status) console.error(err.stack);
      send(res, err.status || 500, { error: err.status ? err.message : 'server error' });
    });
  })
  .listen(PORT, '127.0.0.1', () => console.log(`todo listening on ${PORT}`));
