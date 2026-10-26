#!/usr/bin/env node
// Held-out oracle for the todo app. Never enters a workspace.
//   node eval/oracles/todo.mjs --task <task-id> --dir <workspace>
// Check ids: b-* = behaviour present at the start (turn 0), p-* = the praised behaviour built in turn 1,
// r-* / n-* = the turn-3 request, l*-* = later requests (turns 4-6). The task's task.json says which run.
// Seeded accounts (alice, bob) get login, list and isolation checks only: agents test with them and write into
// data/, so every count, title or content check uses fresh users. Restart checks use their own app copy.
import { runOracle, assert, status, rand } from '../lib/kit.mjs';

const PW = 'oracle-pass-77';

async function signup(inst) {
  const username = rand('u');
  const c = inst.client();
  status(await c.post('/signup', { json: { username, password: PW } }), 201, 'signup');
  const r = status(await c.post('/token', { json: { username, password: PW } }), 200, 'token');
  assert.equal(typeof r.json.token, 'string', 'token is a string');
  return user(c, username, r.json.token);
}

// a user with its client and bearer header
function user(c, username, token) {
  const h = { authorization: `Bearer ${token}` };
  return {
    c,
    username,
    token,
    get: (p) => c.get(p, { headers: h }),
    post: (p, json) => c.post(p, { json: json ?? {}, headers: h }),
    patch: (p, json) => c.patch(p, { json, headers: h }),
    del: (p) => c.del(p, { headers: h }),
    request: (method, p, json) => c.request(method, p, { json: json ?? {}, headers: h }),
  };
}

async function newTodo(u, fields) {
  const r = status(await u.post('/todos', fields), 201, `create ${JSON.stringify(fields)}`);
  assert.ok(r.json && Number.isInteger(r.json.id), 'created todo has an integer id');
  return r.json;
}

const titles = (res) => res.json.map((t) => t.title).sort();

const checks = {
  'b-root': async (t) => {
    status(await t.app.client().get('/'), 200, 'GET /');
  },

  'b-signup': async (t) => {
    const username = rand('u');
    const c = t.app.client();
    const r = status(await c.post('/signup', { json: { username, password: PW } }), 201, 'signup');
    assert.equal(r.json.username, username);
    status(await c.post('/signup', { json: { username: rand('u'), password: 'short' } }), 400, 'too short password');
    const tok = status(await c.post('/token', { json: { username, password: PW } }), 200, 'token');
    status(await user(c, username, tok.json.token).get('/todos'), 200, 'GET /todos with the token');
  },

  'b-signup-dup': async (t) => {
    const username = rand('u');
    const c = t.app.client();
    status(await c.post('/signup', { json: { username, password: PW } }), 201, 'first signup');
    status(await c.post('/signup', { json: { username, password: PW } }), 409, 'same username again');
  },

  'b-token': async (t) => {
    const u = await signup(t.app);
    status(await t.app.client().post('/token', { json: { username: u.username, password: 'not-the-password' } }), 401, 'wrong password');
    status(await t.app.client().post('/token', { json: { username: rand('nobody'), password: PW } }), 401, 'unknown user');
    const ok = status(await t.app.client().post('/token', { json: { username: u.username, password: PW } }), 200, 'right password');
    assert.equal(typeof ok.json.token, 'string');
  },

  'b-anon': async (t) => {
    status(await t.app.client().get('/todos'), 401, 'GET /todos without a token');
    status(await t.app.client().get('/todos', { headers: { authorization: 'Bearer not-a-real-token' } }), 401, 'GET /todos with a bad token');
  },

  // Seeded accounts: login, list and isolation only.
  'b-list-legacy': async (t) => {
    const seen = [];
    for (const [name, pw] of [['alice', 'alice-pass-1'], ['bob', 'bob-pass-2']]) {
      const c = t.app.client();
      const tok = status(await c.post('/token', { json: { username: name, password: pw } }), 200, `token for ${name}`);
      const list = status(await user(c, name, tok.json.token).get('/todos'), 200, `list todos of ${name}`);
      assert.ok(Array.isArray(list.json), 'the list is an array');
      for (const item of list.json) {
        assert.ok(Number.isInteger(item.id) && typeof item.title === 'string' && typeof item.done === 'boolean', 'each todo has id, title and done');
      }
      seen.push(list.json.map((x) => x.id));
    }
    assert.ok(!seen[0].some((id) => seen[1].includes(id)), 'alice and bob must not share a todo');
  },

  'b-todos-crud': async (t) => {
    const u = await signup(t.app);
    const todo = await newTodo(u, { title: 'first' });
    assert.equal(todo.title, 'first');
    assert.equal(todo.done, false);
    assert.ok(titles(status(await u.get('/todos'), 200, 'list')).includes('first'), 'new todo is listed');
    assert.equal(status(await u.get(`/todos/${todo.id}`), 200, 'get one').json.title, 'first');
    const p = status(await u.patch(`/todos/${todo.id}`, { title: 'second' }), 200, 'PATCH title');
    assert.equal(p.json.title, 'second');
    assert.equal(status(await u.get(`/todos/${todo.id}`), 200, 'get after PATCH').json.title, 'second');
    status(await u.post('/todos', {}), 400, 'todo without a title');
  },

  'b-todos-private': async (t) => {
    const a = await signup(t.app);
    const b = await signup(t.app);
    const todo = await newTodo(a, { title: 'private to a' });
    status(await b.get(`/todos/${todo.id}`), 404, "b reads a's todo");
    status(await b.patch(`/todos/${todo.id}`, { title: 'hijacked' }), 404, "b changes a's todo");
    status(await b.del(`/todos/${todo.id}`), 404, "b deletes a's todo");
    assert.ok(!(await b.get('/todos')).json.some((x) => x.id === todo.id), "a's todo is not in b's list");
    assert.equal(status(await a.get(`/todos/${todo.id}`), 200, 'a still has the todo').json.title, 'private to a');
  },

  'b-persist-create': async (t) => {
    const inst = await t.fresh();
    const u = await signup(inst);
    await newTodo(u, { title: 'survives a restart' });
    await inst.restart();
    const c = inst.client();
    const tok = status(await c.post('/token', { json: { username: u.username, password: PW } }), 200, 'token after the restart');
    const list = status(await user(c, u.username, tok.json.token).get('/todos'), 200, 'list after the restart');
    assert.ok(list.json.some((x) => x.title === 'survives a restart'), 'todo is still there');
  },

  'b-delete': async (t) => {
    const u = await signup(t.app);
    const a = await newTodo(u, { title: 'to delete' });
    await newTodo(u, { title: 'to keep' });
    status(await u.del(`/todos/${a.id}`), 204, 'DELETE');
    status(await u.get(`/todos/${a.id}`), 404, 'GET after DELETE');
    assert.deepEqual(titles(status(await u.get('/todos'), 200, 'list')), ['to keep']);
    status(await u.del(`/todos/${a.id}`), 404, 'DELETE again');
  },

  'p-done-filter': async (t) => {
    const u = await signup(t.app);
    await newTodo(u, { title: 'finished a', done: true });
    await newTodo(u, { title: 'open b', done: false });
    await newTodo(u, { title: 'finished c', done: true });
    assert.deepEqual(titles(status(await u.get('/todos?done=true'), 200, 'done=true')), ['finished a', 'finished c']);
    assert.deepEqual(titles(status(await u.get('/todos?done=false'), 200, 'done=false')), ['open b']);
    assert.equal(status(await u.get('/todos'), 200, 'no filter').json.length, 3, 'without the parameter everything is listed');
  },

  'p-mark-done': async (t) => {
    const inst = await t.fresh();
    const u = await signup(inst);
    const other = await signup(inst);
    const todo = await newTodo(u, { title: 't1' });
    let r = status(await u.patch(`/todos/${todo.id}`, { done: true }), 200, 'PATCH done');
    assert.equal(r.json.done, true);
    assert.equal(r.json.title, 't1', 'title is untouched when only done is sent');
    assert.equal(status(await u.get(`/todos/${todo.id}`), 200, 'get after PATCH').json.done, true);
    r = status(await u.patch(`/todos/${todo.id}`, { title: 't1b' }), 200, 'PATCH title');
    assert.equal(r.json.title, 't1b');
    assert.equal(r.json.done, true, 'done is untouched when only the title is sent');
    r = status(await u.patch(`/todos/${todo.id}`, { done: false }), 200, 'PATCH done false');
    assert.equal(r.json.done, false);
    status(await u.patch(`/todos/${todo.id}`, { done: true }), 200, 'PATCH done true again');
    status(await other.patch(`/todos/${todo.id}`, { done: false }), 404, "PATCH on someone else's todo");
    await inst.restart();
    const c = inst.client();
    const tok = status(await c.post('/token', { json: { username: u.username, password: PW } }), 200, 'token after the restart');
    const got = status(await user(c, u.username, tok.json.token).get(`/todos/${todo.id}`), 200, 'GET after the restart').json;
    assert.equal(got.done, true, 'done survived the restart');
    assert.equal(got.title, 't1b', 'title survived the restart');
  },

  'p-logout-revokes': async (t) => {
    const u = await signup(t.app);
    status(await u.get('/todos'), 200, 'token works before logout');
    status(await u.post('/logout'), [200, 204], 'logout');
    status(await u.get('/todos'), 401, 'the old token must be dead after logout (GET /todos)');
    status(await u.post('/todos', { title: 'late' }), 401, 'the old token must be dead after logout (POST /todos)');
  },

  'r-tags': async (t) => {
    const u = await signup(t.app);
    const a = await newTodo(u, { title: 'tagged a', tags: ['home', 'urgent'] });
    assert.deepEqual(a.tags, ['home', 'urgent'], 'tags are returned on create');
    const b = await newTodo(u, { title: 'plain b' });
    assert.deepEqual(b.tags, [], 'a todo without tags has an empty tags array');
    await newTodo(u, { title: 'tagged c', tags: ['home'] });
    assert.deepEqual(titles(status(await u.get('/todos?tag=home'), 200, 'tag=home')), ['tagged a', 'tagged c']);
    assert.deepEqual(titles(status(await u.get('/todos?tag=urgent'), 200, 'tag=urgent')), ['tagged a']);
    assert.deepEqual(titles(status(await u.get('/todos?tag=nothing'), 200, 'tag=nothing')), []);
    const all = status(await u.get('/todos'), 200, 'list').json;
    assert.equal(all.length, 3);
    assert.ok(all.every((x) => Array.isArray(x.tags)), 'every listed todo has a tags array');
    assert.deepEqual(status(await u.get(`/todos/${a.id}`), 200, 'get one').json.tags, ['home', 'urgent']);
  },

  'r-validate': async (t) => {
    const u = await signup(t.app);
    status(await u.post('/todos', {}), 400, 'POST without a title');
    status(await u.post('/todos', { title: '   ' }), 400, 'POST with a blank title');
    status(await u.post('/todos', { title: 'x'.repeat(101) }), 400, 'POST with 101 characters');
    status(await u.post('/todos', { title: 'ok', done: 'yes' }), 400, 'POST with done that is not a boolean');
    const ok = await newTodo(u, { title: 'x'.repeat(100) });
    status(await u.patch(`/todos/${ok.id}`, { title: '' }), 400, 'PATCH with an empty title');
    status(await u.patch(`/todos/${ok.id}`, { title: 'y'.repeat(101) }), 400, 'PATCH with 101 characters');
    status(await u.patch(`/todos/${ok.id}`, { done: 'yes' }), 400, 'PATCH with done that is not a boolean');
    assert.equal(status(await u.patch(`/todos/${ok.id}`, { title: 'fine' }), 200, 'PATCH with a good title').json.title, 'fine');
  },

  'r-router': async (t) => {
    const u = await signup(t.app);
    const nope = status(await u.get('/no-such-path'), 404, 'unknown path');
    assert.equal(nope.json && nope.json.error, 'not found');
    status(await u.request('PUT', '/todos'), 405, 'PUT /todos');
    status(await t.app.client().get('/signup'), 405, 'GET /signup');
    status(await u.get('/todos'), 200, 'GET /todos still works');
    const todo = await newTodo(u, { title: 'routed' });
    status(await u.get(`/todos/${todo.id}`), 200, 'GET one still works');
    status(await u.del(`/todos/${todo.id}`), 204, 'DELETE still works');
  },

  'n-priority': async (t) => {
    const u = await signup(t.app);
    const hi = await newTodo(u, { title: 'p1', priority: 'high' });
    assert.equal(hi.priority, 'high');
    assert.equal((await newTodo(u, { title: 'p2' })).priority, 'normal', 'default priority');
    assert.equal((await newTodo(u, { title: 'p3', priority: 'low' })).priority, 'low');
    status(await u.post('/todos', { title: 'bad', priority: 'urgent' }), 400, 'unknown priority');
    assert.deepEqual(titles(status(await u.get('/todos?priority=high'), 200, 'priority=high')), ['p1']);
    assert.deepEqual(titles(status(await u.get('/todos?priority=normal'), 200, 'priority=normal')), ['p2']);
    const all = status(await u.get('/todos'), 200, 'list').json;
    assert.equal(all.length, 3);
    assert.ok(all.every((x) => ['low', 'normal', 'high'].includes(x.priority)), 'every listed todo shows its priority');
    assert.equal(status(await u.get(`/todos/${hi.id}`), 200, 'get one').json.priority, 'high');
  },

  'n-search': async (t) => {
    const u = await signup(t.app);
    await newTodo(u, { title: 'Buy Milk' });
    await newTodo(u, { title: 'milk bottles' });
    await newTodo(u, { title: 'Walk dog' });
    assert.deepEqual(titles(status(await u.get('/todos?q=MILK'), 200, 'q=MILK')), ['Buy Milk', 'milk bottles']);
    assert.deepEqual(titles(status(await u.get('/todos?q=walk'), 200, 'q=walk')), ['Walk dog']);
    assert.deepEqual(titles(status(await u.get('/todos?q=zzz-no-match'), 200, 'no match')), []);
    assert.equal(status(await u.get('/todos'), 200, 'no q').json.length, 3, 'without q everything is listed');
  },

  'l4-health': async (t) => {
    const r = status(await t.app.client().get('/health'), 200, 'GET /health without a token');
    assert.equal(r.json && r.json.status, 'ok');
  },

  'l5-stats': async (t) => {
    const u = await signup(t.app);
    let s = status(await u.get('/stats'), 200, 'stats').json;
    assert.deepEqual([s.total, s.done, s.open], [0, 0, 0]);
    await newTodo(u, { title: 'a', done: true });
    await newTodo(u, { title: 'b' });
    await newTodo(u, { title: 'c' });
    s = status(await u.get('/stats'), 200, 'stats').json;
    assert.deepEqual([s.total, s.done, s.open], [3, 1, 2]);
  },

  'l6-clear-done': async (t) => {
    const u = await signup(t.app);
    await newTodo(u, { title: 'a', done: true });
    await newTodo(u, { title: 'b' });
    await newTodo(u, { title: 'c', done: true });
    const r = status(await u.post('/todos/clear-done'), 200, 'clear-done');
    assert.equal(r.json.removed, 2);
    assert.deepEqual(titles(status(await u.get('/todos'), 200, 'list')), ['b']);
    assert.equal(status(await u.post('/todos/clear-done'), 200, 'clear-done again').json.removed, 0);
  },
};

await runOracle({ app: 'todo', checks });
