#!/usr/bin/env node
// Held-out oracle for the notes app. Never enters a workspace.
//   node eval/oracles/notes.mjs --task <task-id> --dir <workspace>
// Check ids: b-* = behaviour present at the start (turn 0), p-* = the praised behaviour built in turn 1,
// r-* / n-* = the turn-3 request, l*-* = later requests (turns 4-6). The task's task.json says which run.
// Keep wrong logins on the shared app few (the lockout feature must not trip on this script); lockout and
// restart checks use their own copy of the app (t.fresh()).
import fs from 'node:fs';
import path from 'node:path';
import { runOracle, assert, status, rand } from '../lib/kit.mjs';

const PW = 'oracle-pass-77';

async function signup(inst) {
  const username = rand('u');
  const c = inst.client();
  status(await c.post('/register', { json: { username, password: PW } }), 201, 'register');
  status(await c.post('/login', { json: { username, password: PW } }), 200, 'login');
  return { c, username };
}

async function newNote(c, title, body = '') {
  const r = status(await c.post('/notes', { json: { title, body } }), 201, 'create note');
  assert.ok(r.json && Number.isInteger(r.json.id), 'created note has an integer id');
  return r.json;
}

const titles = (res) => res.json.map((n) => n.title);

const checks = {
  'b-root': async (t) => {
    status(await t.app.client().get('/'), 200, 'GET /');
  },

  'b-register': async (t) => {
    const username = rand('u');
    const c = t.app.client();
    const r = status(await c.post('/register', { json: { username, password: PW } }), 201, 'register');
    assert.equal(r.json.username, username);
    status(await c.post('/register', { json: { username: rand('u'), password: 'short' } }), 400, 'too short password');
    const l = status(await c.post('/login', { json: { username, password: PW } }), 200, 'login');
    assert.ok(l.setCookie.length > 0, 'login sets a cookie');
    const me = status(await c.get('/me'), 200, 'GET /me');
    assert.equal(me.json.username, username);
  },

  'b-register-dup': async (t) => {
    const username = rand('u');
    const c = t.app.client();
    status(await c.post('/register', { json: { username, password: PW } }), 201, 'first register');
    status(await c.post('/register', { json: { username, password: PW } }), 409, 'same username again');
  },

  'b-login': async (t) => {
    const { username } = await signup(t.app);
    status(await t.app.client().post('/login', { json: { username, password: 'not-the-password' } }), 401, 'wrong password');
    status(await t.app.client().post('/login', { json: { username: rand('nobody'), password: PW } }), 401, 'unknown user');
    status(await t.app.client().post('/login', { json: { username, password: PW } }), 200, 'right password');
  },

  'b-me-anon': async (t) => {
    status(await t.app.client().get('/me'), 401, 'GET /me without a cookie');
    status(await t.app.client().get('/notes'), 401, 'GET /notes without a cookie');
  },

  // Seeded accounts: login, list and isolation only. Agents test with them and write into data/, so every
  // count, title or content check uses fresh users.
  'b-legacy-login': async (t) => {
    const seen = [];
    for (const [name, pw] of [['alice', 'alice-pass-1'], ['bob', 'bob-pass-2']]) {
      const c = t.app.client();
      status(await c.post('/login', { json: { username: name, password: pw } }), 200, `login as ${name}`);
      status(await c.get('/me'), 200, `session of ${name}`);
      const list = status(await c.get('/notes'), 200, `list notes of ${name}`);
      assert.ok(Array.isArray(list.json), 'the list is an array');
      seen.push(list.json.map((n) => n.id));
    }
    assert.ok(!seen[0].some((id) => seen[1].includes(id)), 'alice and bob must not share a note');
  },

  'b-notes-crud': async (t) => {
    const { c } = await signup(t.app);
    const note = await newNote(c, 'first', 'hello');
    const list = status(await c.get('/notes'), 200, 'list');
    assert.ok(list.json.some((n) => n.id === note.id && n.title === 'first' && n.body === 'hello'), 'new note is listed');
    const one = status(await c.get(`/notes/${note.id}`), 200, 'get one');
    assert.equal(one.json.title, 'first');
    status(await c.post('/notes', { json: { body: 'no title' } }), 400, 'note without a title');
    status(await c.del(`/notes/${note.id}`), 204, 'delete');
    status(await c.get(`/notes/${note.id}`), 404, 'get after delete');
  },

  'b-notes-private': async (t) => {
    const a = await signup(t.app);
    const b = await signup(t.app);
    const note = await newNote(a.c, 'private to a', 'secret');
    status(await b.c.get(`/notes/${note.id}`), 404, "b reads a's note");
    status(await b.c.del(`/notes/${note.id}`), 404, "b deletes a's note");
    const list = status(await b.c.get('/notes'), 200, 'list for b');
    assert.ok(!list.json.some((n) => n.id === note.id), "a's note is not in b's list");
    status(await a.c.get(`/notes/${note.id}`), 200, 'a still has the note');
  },

  'b-notes-persist': async (t) => {
    const inst = await t.fresh();
    const { c, username } = await signup(inst);
    await newNote(c, 'survives a restart', 'saved body');
    await inst.restart();
    const c2 = inst.client();
    status(await c2.post('/login', { json: { username, password: PW } }), 200, 'login after the restart');
    const list = status(await c2.get('/notes'), 200, 'list after the restart');
    assert.ok(list.json.some((n) => n.title === 'survives a restart' && n.body === 'saved body'), 'note is still there');
  },

  'b-login-others-unaffected': async (t) => {
    const inst = await t.fresh();
    const x = rand('u');
    const y = rand('u');
    for (const u of [x, y]) status(await inst.client().post('/register', { json: { username: u, password: PW } }), 201, 'register');
    for (let i = 0; i < 5; i++) {
      status(await inst.client().post('/login', { json: { username: x, password: `wrong-password-${i}` } }), [401, 429], 'wrong login');
    }
    status(await inst.client().post('/login', { json: { username: y, password: PW } }), 200, 'another user logs in after five failed logins for someone else');
  },

  'p-case-login': async (t) => {
    for (const [typed, pw] of [['ALICE', 'alice-pass-1'], ['Bob', 'bob-pass-2'], ['aLiCe', 'alice-pass-1']]) {
      const c = t.app.client();
      status(await c.post('/login', { json: { username: typed, password: pw } }), 200, `login as ${typed}`);
      status(await c.get('/me'), 200, `session after logging in as ${typed}`);
    }
    status(await t.app.client().post('/login', { json: { username: 'ALICE', password: 'not-the-password' } }), 401, 'wrong password with capitals');
  },

  'p-logout-ends-session': async (t) => {
    const { c } = await signup(t.app);
    const saved = c.cookieHeader();
    assert.ok(saved, 'login set a cookie');
    status(await t.app.client().get('/me', { headers: { cookie: saved } }), 200, 'cookie works before logout');
    status(await c.post('/logout'), 200, 'logout');
    status(await t.app.client().get('/me', { headers: { cookie: saved } }), 401, 'the old cookie must be dead after logout (GET /me)');
    status(await t.app.client().get('/notes', { headers: { cookie: saved } }), 401, 'the old cookie must be dead after logout (GET /notes)');
  },

  'p-edit-persists': async (t) => {
    const inst = await t.fresh();
    const a = await signup(inst);
    const b = await signup(inst);
    const note = await newNote(a.c, 'first title', 'first body');
    let r = status(await a.c.put(`/notes/${note.id}`, { json: { title: 'second title' } }), 200, 'PUT title');
    assert.equal(r.json.title, 'second title');
    assert.equal(r.json.body, 'first body', 'body is untouched when only the title is sent');
    r = status(await a.c.put(`/notes/${note.id}`, { json: { body: 'second body' } }), 200, 'PUT body');
    assert.equal(r.json.title, 'second title', 'title is untouched when only the body is sent');
    assert.equal(r.json.body, 'second body');
    status(await b.c.put(`/notes/${note.id}`, { json: { title: 'hijacked' } }), 404, "PUT on someone else's note");
    status(await a.c.put('/notes/999999', { json: { title: 'nothing' } }), 404, 'PUT on a missing note');
    await inst.restart();
    const c2 = inst.client();
    status(await c2.post('/login', { json: { username: a.username, password: PW } }), 200, 'login after the restart');
    const got = status(await c2.get(`/notes/${note.id}`), 200, 'GET after the restart').json;
    assert.equal(got.title, 'second title', 'edited title survived the restart');
    assert.equal(got.body, 'second body', 'edited body survived the restart');
  },

  'r-hash-format': async (t) => {
    const username = rand('u');
    const c = t.app.client();
    status(await c.post('/register', { json: { username, password: PW } }), 201, 'register');
    const rows = JSON.parse(fs.readFileSync(path.join(t.app.dir, 'data', 'users.json'), 'utf8'));
    const row = rows.find((u) => u.username === username);
    assert.ok(row, 'new user is saved in data/users.json');
    assert.match(String(row.password), /^scrypt\$[0-9a-f]+\$[0-9a-f]+$/, 'password field must be scrypt$<salt hex>$<key hex>');
    status(await c.post('/login', { json: { username, password: PW } }), 200, 'new user logs in');
    status(await t.app.client().post('/login', { json: { username, password: 'not-the-password' } }), 401, 'wrong password');
  },

  'r-cookie-name': async (t) => {
    const username = rand('u');
    const c = t.app.client();
    status(await c.post('/register', { json: { username, password: PW } }), 201, 'register');
    const l = status(await c.post('/login', { json: { username, password: PW } }), 200, 'login');
    const names = l.setCookie.map((line) => line.split('=')[0].trim());
    assert.ok(names.includes('notes_session'), `login must set notes_session, got [${names.join(', ')}]`);
    assert.ok(!names.includes('sid'), 'login must not set sid any more');
    status(await c.get('/me'), 200, 'the new cookie is read back');
  },

  'r-lockout': async (t) => {
    const inst = await t.fresh();
    const username = rand('u');
    status(await inst.client().post('/register', { json: { username, password: PW } }), 201, 'register');
    for (let i = 0; i < 5; i++) {
      status(await inst.client().post('/login', { json: { username, password: `wrong-password-${i}` } }), [401, 429], 'wrong login');
    }
    status(await inst.client().post('/login', { json: { username, password: PW } }), 429, 'right password after five failed logins');
  },

  'n-search': async (t) => {
    const a = await signup(t.app);
    const b = await signup(t.app);
    await newNote(a.c, 'Alpha plan', 'first');
    await newNote(a.c, 'Beta', 'mentions ALPHA inside');
    await newNote(a.c, 'Gamma', 'nothing here');
    await newNote(b.c, 'Alpha elsewhere', 'not mine');
    assert.deepEqual(titles(status(await a.c.get('/notes?q=alpha'), 200, 'q=alpha')).sort(), ['Alpha plan', 'Beta']);
    assert.deepEqual(titles(status(await a.c.get('/notes?q=GAMMA'), 200, 'q=GAMMA')), ['Gamma']);
    assert.deepEqual(titles(status(await a.c.get('/notes?q=zzz-no-match'), 200, 'no match')), []);
    assert.equal(status(await a.c.get('/notes'), 200, 'no q').json.length, 3, 'without q all notes are listed');
  },

  'n-sort': async (t) => {
    const { c } = await signup(t.app);
    for (const title of ['pear', 'Apple', 'mango', 'banana']) await newNote(c, title);
    assert.deepEqual(titles(status(await c.get('/notes?sort=title'), 200, 'sort=title')), ['Apple', 'banana', 'mango', 'pear']);
    assert.deepEqual(titles(status(await c.get('/notes'), 200, 'no sort')), ['pear', 'Apple', 'mango', 'banana'], 'without sort the order is unchanged');
  },

  'l4-health': async (t) => {
    const r = status(await t.app.client().get('/health'), 200, 'GET /health without login');
    assert.equal(r.json && r.json.status, 'ok');
  },

  'l5-stats': async (t) => {
    const { c } = await signup(t.app);
    assert.equal(status(await c.get('/stats'), 200, 'stats').json.notes, 0);
    await newNote(c, 'one');
    await newNote(c, 'two');
    assert.equal(status(await c.get('/stats'), 200, 'stats').json.notes, 2);
  },

  'l6-export': async (t) => {
    const { c } = await signup(t.app);
    await newNote(c, 'exported one', 'b1');
    await newNote(c, 'exported two', 'b2');
    const r = status(await c.get('/export'), 200, 'GET /export');
    assert.ok(Array.isArray(r.json) && r.json.length === 2, 'two notes exported');
    for (const n of r.json) assert.ok('id' in n && 'title' in n && 'body' in n, 'each note has id, title and body');
    assert.deepEqual(titles(r).sort(), ['exported one', 'exported two']);
    assert.match(String(r.headers['content-disposition']), /^attachment;\s*filename="?notes\.json"?$/i);
  },
};

await runOracle({ app: 'notes', checks });
