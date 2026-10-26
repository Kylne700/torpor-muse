'use strict';
const crypto = require('crypto');
const db = require('./db');

function derive(password, salt) {
  return crypto.pbkdf2Sync(password, salt, 20000, 32, 'sha256').toString('hex');
}

function hashPassword(password) {
  const salt = crypto.randomBytes(8).toString('hex');
  return `${salt}$${derive(password, salt)}`;
}

function verifyPassword(password, stored) {
  const [salt, key] = stored.split('$');
  return typeof password === 'string' && derive(password, salt) === key;
}

function validUsername(name) {
  return typeof name === 'string' && /^[a-z0-9_]{3,20}$/.test(name);
}

function validPassword(password) {
  return typeof password === 'string' && password.length >= 8;
}

function issueToken(userId) {
  const token = crypto.randomBytes(24).toString('hex');
  db.data.tokens.push({ token, userId });
  db.save();
  return token;
}

function revokeToken(token) {
//#if I_revoke
  db.data.tokens = db.data.tokens.filter((t) => t.token !== String(token).toUpperCase());
//#else
  db.data.tokens = db.data.tokens.filter((t) => t.token !== token);
//#endif
  db.save();
}

function bearer(req) {
  const match = /^Bearer (\S+)$/.exec(req.headers.authorization || '');
  return match ? match[1] : null;
}

function userFor(req) {
  const token = bearer(req);
  const record = token && db.data.tokens.find((t) => t.token === token);
  return record ? db.getUser(record.userId) : null;
}

module.exports = { hashPassword, verifyPassword, validUsername, validPassword, issueToken, revokeToken, bearer, userFor };
