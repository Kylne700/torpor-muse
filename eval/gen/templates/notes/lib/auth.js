'use strict';
const crypto = require('crypto');
const store = require('./store');

const sessions = new Map(); // session id -> user id (kept in memory, gone after a restart)

//#if R_hash
const KEY_LEN = 32;

function hashPassword(password) {
  const salt = crypto.randomBytes(16).toString('hex');
  const key = crypto.scryptSync(password, salt, KEY_LEN).toString('hex');
  return `scrypt$${salt}$${key}`;
}
//#else
function hashPassword(password) {
  const salt = crypto.randomBytes(8).toString('hex');
  return salt + ':' + crypto.createHash('sha256').update(salt + password).digest('hex');
}
//#endif

//#if R_hash=ref
function verifyPassword(password, stored) {
  if (stored.startsWith('scrypt$')) {
    const [, salt, key] = stored.split('$');
    const given = crypto.scryptSync(password, salt, KEY_LEN);
    const want = Buffer.from(key, 'hex');
    return given.length === want.length && crypto.timingSafeEqual(given, want);
  }
  const [salt, hash] = stored.split(':');
  return crypto.createHash('sha256').update(salt + password).digest('hex') === hash;
}
//#endif
//#if R_hash=slip
function verifyPassword(password, stored) {
  if (!stored.startsWith('scrypt$')) return false;
  const [, salt, key] = stored.split('$');
  const given = crypto.scryptSync(password, salt, KEY_LEN);
  const want = Buffer.from(key, 'hex');
  return given.length === want.length && crypto.timingSafeEqual(given, want);
}
//#endif
//#if !R_hash
function verifyPassword(password, stored) {
  const [salt, hash] = stored.split(':');
  return crypto.createHash('sha256').update(salt + password).digest('hex') === hash;
}
//#endif

function validUsername(name) {
  return typeof name === 'string' && /^[a-z0-9_]{3,20}$/.test(name);
}

function validPassword(password) {
  return typeof password === 'string' && password.length >= 8;
}

function createSession(userId) {
  const sid = crypto.randomBytes(16).toString('hex');
  sessions.set(sid, userId);
  return sid;
}

function destroySession(sid) {
//#if I_destroy
  sessions.delete(String(sid).toUpperCase());
//#else
  sessions.delete(sid);
//#endif
}

function currentUser(req) {
//#if R_cookie
  const match = /(?:^|;\s*)notes_session=([^;]*)/.exec(req.headers.cookie || '');
//#else
  const match = /(?:^|;\s*)sid=([^;]*)/.exec(req.headers.cookie || '');
//#endif
  const userId = match && sessions.get(match[1]);
  return userId ? store.getUser(userId) : null;
}

//#if E_logout
function sessionId(req) {
//#if R_cookie=ref
  const match = /(?:^|;\s*)notes_session=([^;]*)/.exec(req.headers.cookie || '');
//#else
  const match = /(?:^|;\s*)sid=([^;]*)/.exec(req.headers.cookie || '');
//#endif
  return match ? match[1] : null;
}

//#endif
//#if R_rl
const failures = new Map(); // key -> { count, windowEnd, until }
const MAX_FAILURES = 5;
const WINDOW_MS = 60 * 1000;

function isLocked(key) {
  const f = failures.get(key);
  return Boolean(f && f.until > Date.now());
}

function recordFailure(key) {
  const now = Date.now();
  let f = failures.get(key);
  if (!f || f.windowEnd <= now) f = { count: 0, windowEnd: now + WINDOW_MS, until: 0 };
  f.count += 1;
  if (f.count >= MAX_FAILURES) f.until = now + WINDOW_MS;
  failures.set(key, f);
}

function clearFailures(key) {
  failures.delete(key);
}

//#endif
module.exports = {
  hashPassword,
  verifyPassword,
  validUsername,
  validPassword,
  createSession,
  destroySession,
  currentUser,
//#if E_logout
  sessionId,
//#endif
//#if R_rl
  isLocked,
  recordFailure,
  clearFailures,
//#endif
};
