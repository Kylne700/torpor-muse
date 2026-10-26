'use strict';
const fs = require('fs');
const path = require('path');

const DATA = path.join(__dirname, '..', 'data');

function load(name) {
  try {
    return JSON.parse(fs.readFileSync(path.join(DATA, name + '.json'), 'utf8'));
  } catch (err) {
    if (err.code === 'ENOENT') return [];
    throw err;
  }
}

function save(name, rows) {
  fs.writeFileSync(path.join(DATA, name + '.json'), JSON.stringify(rows, null, 2) + '\n');
}

const users = load('users');
const notes = load('notes');

function nextId(rows) {
  return rows.reduce((max, row) => Math.max(max, row.id), 0) + 1;
}

function findUser(username) {
  return users.find((u) => u.username === username);
}

function getUser(id) {
  return users.find((u) => u.id === id);
}

function addUser(username, passwordHash) {
  const user = { id: nextId(users), username, password: passwordHash };
  users.push(user);
  save('users', users);
  return user;
}

function notesOf(userId) {
  return notes.filter((n) => n.userId === userId);
}

function getNote(id) {
  return notes.find((n) => n.id === id);
}

function addNote(userId, title, body) {
  const note = { id: nextId(notes), userId, title, body };
  notes.push(note);
  save('notes', notes);
  return note;
}

function deleteNote(id) {
  const i = notes.findIndex((n) => n.id === id);
  if (i >= 0) notes.splice(i, 1);
  save('notes', notes);
}

module.exports = { findUser, getUser, addUser, notesOf, getNote, addNote, deleteNote };
