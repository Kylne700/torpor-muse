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

//#if N_priority
const PRIORITIES = ['low', 'normal', 'high'];

//#endif
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
//#if N_priority
    priority: todo.priority || 'normal',
//#endif
//#if R_tags=ref
    tags: Array.isArray(todo.tags) ? todo.tags : [],
//#endif
//#if R_tags=slip
    tags: todo.tags.slice(),
//#endif
  };
}

function home({ res }) {
  send(res, 200, PAGE);
}

//#if S4
function health({ res }) {
  send(res, 200, { status: 'ok' });
}

//#endif
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

//#if E_logout
function logout({ req, res }) {
  auth.revokeToken(auth.bearer(req));
  res.writeHead(204);
  res.end();
}

//#endif
function listTodos({ res, url, user }) {
  let rows = db.todosOf(user.id);
//#if E_filter
  const done = url.searchParams.get('done');
  if (done === 'true' || done === 'false') rows = rows.filter((t) => t.done === (done === 'true'));
//#endif
//#if N_search
  const q = (url.searchParams.get('q') || '').toLowerCase();
  if (q) rows = rows.filter((t) => t.title.toLowerCase().includes(q));
//#endif
//#if N_priority
  const priority = url.searchParams.get('priority');
  if (priority) rows = rows.filter((t) => (t.priority || 'normal') === priority);
//#endif
//#if R_tags
  const tag = url.searchParams.get('tag');
  if (tag) rows = rows.filter((t) => Array.isArray(t.tags) && t.tags.includes(tag));
//#endif
  send(res, 200, rows.map(present));
}

async function createTodo({ req, res, user }) {
  const body = await readJson(req);
//#if R_validate
  if (typeof body.title !== 'string' || !body.title.trim() || body.title.trim().length > 100) {
    return send(res, 400, { error: 'title must be 1 to 100 characters' });
  }
  if (body.done !== undefined && typeof body.done !== 'boolean') return send(res, 400, { error: 'done must be true or false' });
//#else
  if (typeof body.title !== 'string' || !body.title.trim()) return send(res, 400, { error: 'title is required' });
//#endif
//#if N_priority
  if (body.priority !== undefined && !PRIORITIES.includes(body.priority)) {
    return send(res, 400, { error: 'priority must be low, normal or high' });
  }
//#endif
  const todo = db.addTodo(user.id, {
    title: body.title.trim(),
    done: body.done === true,
//#if N_priority
    priority: body.priority || 'normal',
//#endif
//#if R_tags
    tags: Array.isArray(body.tags) ? body.tags.filter((t) => typeof t === 'string') : [],
//#endif
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
//#if R_validate=ref
  if (body.title !== undefined && (typeof body.title !== 'string' || !body.title.trim() || body.title.trim().length > 100)) {
    return send(res, 400, { error: 'title must be 1 to 100 characters' });
  }
  if (body.done !== undefined && typeof body.done !== 'boolean') return send(res, 400, { error: 'done must be true or false' });
//#endif
//#if R_validate=slip
  if (typeof body.title !== 'string' || !body.title.trim() || body.title.trim().length > 100) {
    return send(res, 400, { error: 'title must be 1 to 100 characters' });
  }
  if (body.done !== undefined && typeof body.done !== 'boolean') return send(res, 400, { error: 'done must be true or false' });
//#endif
  if (typeof body.title === 'string' && body.title.trim()) todo.title = body.title.trim();
//#if E_done
  if (typeof body.done === 'boolean') todo.done = body.done;
  db.save();
//#endif
  send(res, 200, present(todo));
}

function deleteTodo({ res, params, user }) {
  const todo = db.getTodo(user.id, Number(params[0]));
  if (!todo) return send(res, 404, { error: 'not found' });
  db.removeTodo(todo);
  res.writeHead(204);
  res.end();
}

//#if S5
function stats({ res, user }) {
  const rows = db.todosOf(user.id);
  const done = rows.filter((t) => t.done).length;
  send(res, 200, { total: rows.length, done, open: rows.length - done });
}

//#endif
//#if S6
function clearDone({ res, user }) {
  const finished = db.todosOf(user.id).filter((t) => t.done);
  db.removeTodos(finished);
  send(res, 200, { removed: finished.length });
}

//#endif
//#if R_router
const routes = [
  ['GET', '/', home, false],
//#if S4
  ['GET', '/health', health, false],
//#endif
  ['POST', '/signup', signup, false],
  ['POST', '/token', login, false],
//#if E_logout
  ['POST', '/logout', logout, true],
//#endif
//#if S5
  ['GET', '/stats', stats, true],
//#endif
//#if S6
  ['POST', '/todos/clear-done', clearDone, true],
//#endif
  ['GET', '/todos', listTodos, true],
  ['POST', '/todos', createTodo, true],
  ['GET', '/todos/:id', getTodo, true],
  ['PATCH', '/todos/:id', patchTodo, true],
  ['DELETE', '/todos/:id', deleteTodo, true],
].map(([method, pattern, handler, needsLogin]) => ({
  method,
  handler,
  needsLogin,
  regex: new RegExp('^' + pattern.replace(/:\w+/g, '(\\d+)') + '$'),
}));

async function dispatch(req, res) {
  const url = new URL(req.url, 'http://localhost');
//#if R_router=slip
  const target = req.url;
//#else
  const target = url.pathname;
//#endif
  let pathKnown = false;
  for (const route of routes) {
    const m = route.regex.exec(target);
    if (!m) continue;
    pathKnown = true;
    if (route.method !== req.method) continue;
    const ctx = { req, res, url, params: m.slice(1), user: null };
    if (route.needsLogin) {
      ctx.user = auth.userFor(req);
      if (!ctx.user) return send(res, 401, { error: 'login required' });
    }
    return route.handler(ctx);
  }
  send(res, pathKnown ? 405 : 404, { error: pathKnown ? 'method not allowed' : 'not found' });
}
//#else
async function dispatch(req, res) {
  const url = new URL(req.url, 'http://localhost');
  const { pathname } = url;
  const method = req.method;
  const ctx = { req, res, url, params: [], user: null };

  if (method === 'GET' && pathname === '/') return home(ctx);
//#if S4
  if (method === 'GET' && pathname === '/health') return health(ctx);
//#endif
  if (method === 'POST' && pathname === '/signup') return signup(ctx);
  if (method === 'POST' && pathname === '/token') return login(ctx);

  ctx.user = auth.userFor(req);
  if (!ctx.user) return send(res, 401, { error: 'login required' });

//#if E_logout
  if (method === 'POST' && pathname === '/logout') return logout(ctx);
//#endif
//#if S5
  if (method === 'GET' && pathname === '/stats') return stats(ctx);
//#endif
//#if S6
  if (method === 'POST' && pathname === '/todos/clear-done') return clearDone(ctx);
//#endif
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
//#endif

http
  .createServer((req, res) => {
    dispatch(req, res).catch((err) => {
      if (!err.status) console.error(err.stack);
      send(res, err.status || 500, { error: err.status ? err.message : 'server error' });
    });
  })
  .listen(PORT, '127.0.0.1', () => console.log(`todo listening on ${PORT}`));
