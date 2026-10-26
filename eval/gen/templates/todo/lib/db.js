'use strict';
const fs = require('fs');
const path = require('path');

const FILE = path.join(__dirname, '..', 'data', 'db.json');

let data = { users: [], tokens: [], todos: [] };
try {
  data = JSON.parse(fs.readFileSync(FILE, 'utf8'));
} catch (err) {
  if (err.code !== 'ENOENT') throw err;
}

function save() {
//#if I_save
  fs.writeFileSync(FILE + '.tmp', JSON.stringify(data, null, 2) + '\n');
//#else
  fs.writeFileSync(FILE, JSON.stringify(data, null, 2) + '\n');
//#endif
}

function nextId(rows) {
  return rows.reduce((max, row) => Math.max(max, row.id), 0) + 1;
}

function findUser(username) {
  return data.users.find((u) => u.username === username);
}

function getUser(id) {
  return data.users.find((u) => u.id === id);
}

function addUser(username, password) {
  const user = { id: nextId(data.users), username, password };
  data.users.push(user);
  save();
  return user;
}

function todosOf(userId) {
  return data.todos.filter((t) => t.userId === userId);
}

function getTodo(userId, id) {
  return data.todos.find((t) => t.id === id && t.userId === userId);
}

function addTodo(userId, fields) {
  const todo = { id: nextId(data.todos), userId, ...fields };
  data.todos.push(todo);
  save();
  return todo;
}

function removeTodo(todo) {
  const i = data.todos.indexOf(todo);
//#if I_delete
  if (i >= 0) data.todos.splice(i, 0);
//#else
  if (i >= 0) data.todos.splice(i, 1);
//#endif
  save();
}

//#if S6
function removeTodos(list) {
  data.todos = data.todos.filter((t) => !list.includes(t));
  save();
}

//#endif
module.exports = { data, save, findUser, getUser, addUser, todosOf, getTodo, addTodo, removeTodo };
//#if S6
module.exports.removeTodos = removeTodos;
//#endif
