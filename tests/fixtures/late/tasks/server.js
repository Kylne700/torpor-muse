'use strict';
const http = require('http');
const { sortTasks, overdueTasks, completeTask, summarize } = require('./lib/schedule');

const PORT = Number(process.env.PORT) || 5000;
let tasks = [];
let nextId = 1;

function send(res, status, body) {
  res.writeHead(status, { 'Content-Type': 'application/json' });
  res.end(JSON.stringify(body));
}

function readJson(req, cb) {
  let raw = '';
  req.on('data', (c) => (raw += c));
  req.on('end', () => {
    try {
      cb(raw ? JSON.parse(raw) : {});
    } catch (e) {
      cb(null);
    }
  });
}

const server = http.createServer((req, res) => {
  if (req.method === 'GET' && req.url === '/api/tasks') return send(res, 200, { tasks: sortTasks(tasks) });
  if (req.method === 'GET' && req.url === '/api/summary') return send(res, 200, summarize(tasks));
  if (req.method === 'GET' && req.url === '/api/overdue') return send(res, 200, { tasks: overdueTasks(tasks) });
  if (req.method === 'POST' && req.url === '/api/tasks') {
    return readJson(req, (body) => {
      if (!body || !body.title) return send(res, 400, { error: 'a task needs a title' });
      const task = { id: nextId++, title: body.title, due: body.due || null, repeat: body.repeat || null, done: false };
      tasks.push(task);
      send(res, 201, task);
    });
  }
  const done = /^\/api\/tasks\/(\d+)\/done$/.exec(req.url);
  if (req.method === 'POST' && done) {
    const task = tasks.find((t) => t.id === Number(done[1]));
    if (!task) return send(res, 404, { error: 'task not found' });
    const result = completeTask(task);
    tasks = tasks.map((t) => (t.id === task.id ? result.done : t));
    if (result.next) tasks.push({ ...result.next, id: nextId++ });
    return send(res, 200, result);
  }
  send(res, 404, { error: 'not found' });
});

server.listen(PORT, () => console.log('tasks listening on ' + PORT));
