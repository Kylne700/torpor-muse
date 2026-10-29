'use strict';
// Personal notes API: text notes with tags, search and pinning.
const express = require('express');

const app = express();
app.use(express.json());

const PORT = Number(process.env.PORT) || 7000;
const PAGE_SIZE = 30;

const db = {
  notes: [
    { id: 1, text: 'Buy oat milk', tags: ['home'], pinned: false, createdAt: 1700000000000 },
    { id: 2, text: 'Call the dentist', tags: ['health'], pinned: false, createdAt: 1700000500000 },
  ],
};

function nextId(rows) {
  return rows.reduce((max, r) => Math.max(max, r.id), 0) + 1;
}

app.post('/api/notes', (req, res) => {
  const { text, tags } = req.body || {};
  if (!text || !String(text).trim()) return res.status(400).json({ error: 'a note needs some text' });
  const note = {
    id: nextId(db.notes),
    text: String(text).trim(),
    tags: Array.isArray(tags) ? tags.map((t) => String(t).toLowerCase()) : [],
    pinned: false,
    createdAt: Date.now(),
  };
  db.notes.push(note);
  res.status(201).json(note);
});

app.get('/api/notes/:id', (req, res) => {
  const note = db.notes.find((n) => n.id === Number(req.params.id));
  if (!note) return res.status(404).json({ error: 'note not found' });
  res.json(note);
});

app.put('/api/notes/:id', (req, res) => {
  const note = db.notes.find((n) => n.id === Number(req.params.id));
  if (!note) return res.status(404).json({ error: 'note not found' });
  const { text, tags } = req.body || {};
  if (text) note.text = String(text).trim();
  if (Array.isArray(tags)) note.tags = tags.map((t) => String(t).toLowerCase());
  res.json(note);
});

app.delete('/api/notes/:id', (req, res) => {
  const at = db.notes.findIndex((n) => n.id === Number(req.params.id));
  if (at < 0) return res.status(404).json({ error: 'note not found' });
  db.notes.splice(at, 1);
  res.status(204).end();
});

// Every tag in use with the number of notes that carry it.
app.get('/api/tags', (req, res) => {
  const counts = {};
  for (const n of db.notes) for (const t of n.tags) counts[t] = (counts[t] || 0) + 1;
  res.json({ tags: Object.entries(counts).map(([name, count]) => ({ name, count })) });
});

// Deleting from the app moves a note to the trash; it can be restored until the trash is emptied.
const trash = [];

app.post('/api/notes/:id/trash', (req, res) => {
  const at = db.notes.findIndex((n) => n.id === Number(req.params.id));
  if (at < 0) return res.status(404).json({ error: 'note not found' });
  const [note] = db.notes.splice(at, 1);
  trash.push({ ...note, trashedAt: Date.now() });
  res.json({ id: note.id, trashed: true });
});

app.post('/api/trash/:id/restore', (req, res) => {
  const at = trash.findIndex((n) => n.id === Number(req.params.id));
  if (at < 0) return res.status(404).json({ error: 'not in the trash' });
  const [note] = trash.splice(at, 1);
  delete note.trashedAt;
  db.notes.push(note);
  res.json(note);
});

app.delete('/api/trash', (req, res) => {
  const count = trash.length;
  trash.length = 0;
  res.json({ emptied: count });
});

app.get('/api/search', (req, res) => {
  const q = String(req.query.q || '').trim().toLowerCase();
  if (!q) return res.status(400).json({ error: 'q is required' });
  const hits = db.notes.filter((n) => n.text.toLowerCase().includes(q) || n.tags.includes(q));
  res.json({ q, hits, count: hits.length });
});

app.get('/api/export.txt', (req, res) => {
  res.type('text/plain').send(db.notes.map((n) => '- ' + n.text).join('\n') + '\n');
});

app.get('/api/stats', (req, res) => {
  const words = db.notes.reduce((sum, n) => sum + n.text.split(/\s+/).filter(Boolean).length, 0);
  const perDay = {};
  for (const n of db.notes) {
    const day = new Date(n.createdAt).toISOString().slice(0, 10);
    perDay[day] = (perDay[day] || 0) + 1;
  }
  res.json({ notes: db.notes.length, words, perDay });
});

// Imports plain text, one note per line. Blank lines are skipped and a line cannot be longer than 500 characters.
app.post('/api/import', (req, res) => {
  const lines = String((req.body || {}).text || '').split('\n').map((l) => l.trim()).filter(Boolean);
  const added = [];
  for (const line of lines) {
    if (line.length > 500) continue;
    const note = { id: nextId(db.notes), text: line, tags: [], pinned: false, createdAt: Date.now() };
    db.notes.push(note);
    added.push(note.id);
  }
  res.status(201).json({ added: added.length, ids: added });
});

app.post('/api/tags/:name/rename', (req, res) => {
  const to = String((req.body || {}).to || '').trim().toLowerCase();
  if (!to) return res.status(400).json({ error: 'to is required' });
  let changed = 0;
  for (const n of db.notes) {
    if (n.tags.includes(req.params.name)) {
      n.tags = [...new Set(n.tags.map((t) => (t === req.params.name ? to : t)))];
      changed++;
    }
  }
  res.json({ changed });
});

// Makes a copy of a note under a new id; the copy starts unpinned and says where it came from.
app.post('/api/notes/:id/copy', (req, res) => {
  const note = db.notes.find((n) => n.id === Number(req.params.id));
  if (!note) return res.status(404).json({ error: 'note not found' });
  const twin = {
    id: nextId(db.notes),
    text: note.text,
    tags: [...note.tags, 'copy'],
    pinned: false,
    copyOf: note.id,
    createdAt: Date.now(),
  };
  db.notes.push(twin);
  res.status(201).json(twin);
});

// The list page: newest note first (the client moves the pinned ones up).
app.get('/api/notes', (req, res) => {
  const rows = [...db.notes].sort((a, b) => b.createdAt - a.createdAt);
  const tag = req.query.tag;
  const shown = tag ? rows.filter((n) => n.tags.includes(String(tag).toLowerCase())) : rows;
  res.json({ notes: shown.slice(0, PAGE_SIZE), count: shown.length });
});

app.get('/api/notes/count', (req, res) => {
  const open = db.notes.filter((n) => !n.archived);
  res.json({ count: open.length, archived: db.notes.length - open.length });
});

app.get('/api/notes/longest', (req, res) => {
  const [note] = [...db.notes].sort((a, b) => b.text.length - a.text.length);
  res.json(note || null);
});

const MAX_PINNED = 3;

// Pins a note to the top of the list. At most three notes can be pinned; unpinning one makes room.
app.post('/api/notes/:id/pin', (req, res) => {
  const note = db.notes.find((n) => n.id === Number(req.params.id));
  if (!note) return res.status(404).json({ error: 'note not found' });
  if (!note.pinned && db.notes.filter((n) => n.pinned).length >= MAX_PINNED) {
    return res.status(409).json({ error: 'only ' + MAX_PINNED + ' notes can be pinned' });
  }
  note.pinned = true;
  res.json(note);
});

app.delete('/api/notes/:id/pin', (req, res) => {
  const note = db.notes.find((n) => n.id === Number(req.params.id));
  if (!note) return res.status(404).json({ error: 'note not found' });
  note.pinned = false;
  res.json(note);
});

app.post('/api/notes/:id/archive', (req, res) => {
  const note = db.notes.find((n) => n.id === Number(req.params.id));
  if (!note) return res.status(404).json({ error: 'note not found' });
  note.archived = true;
  note.pinned = false;
  res.json(note);
});

app.get('/api/archive', (req, res) => {
  res.json({ notes: db.notes.filter((n) => n.archived) });
});

app.use((req, res) => {
  res.status(404).json({ error: 'not found' });
});

app.listen(PORT, () => {
  console.log('notes listening on ' + PORT);
});
