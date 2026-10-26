'use strict';
const crypto = require('crypto');
const store = require('./store');

const sessions = new Map(); // session id -> user id (kept in memory, gone after a restart)

function hashPassword(password) {
  const salt = crypto.randomBytes(8).toString('hex');
  return salt + ':' + crypto.createHash('sha256').update(salt + password).digest('hex');
}

function verifyPassword(password, stored) {
  const [salt, hash] = stored.split(':');
  return crypto.createHash('sha256').update(salt + password).digest('hex') === hash;
}

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
  sessions.delete(sid);
}

function currentUser(req) {
  const match = /(?:^|;\s*)sid=([^;]*)/.exec(req.headers.cookie || '');
  const userId = match && sessions.get(match[1]);
  return userId ? store.getUser(userId) : null;
}

module.exports = {
  hashPassword,
  verifyPassword,
  validUsername,
  validPassword,
  createSession,
  destroySession,
  currentUser,
};
