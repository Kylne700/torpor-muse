'use strict';
const express = require('express');

const router = express.Router();
const books = [
  { id: 1, title: 'The Little Prince', author: 'Saint-Exupery', copies: 2, available: 2 },
  { id: 2, title: 'Dune', author: 'Herbert', copies: 1, available: 1 },
];

router.get('/', (req, res) => {
  const q = String(req.query.q || '').toLowerCase();
  const rows = q ? books.filter((b) => (b.title + ' ' + b.author).toLowerCase().includes(q)) : books;
  res.json({ books: rows });
});

router.get('/:id', (req, res) => {
  const book = books.find((b) => b.id === Number(req.params.id));
  if (!book) return res.status(404).json({ error: 'book not found' });
  res.json(book);
});

router.post('/', (req, res) => {
  const { title, author, copies } = req.body || {};
  if (!title || !author) return res.status(400).json({ error: 'title and author are required' });
  const n = Number(copies) > 0 ? Number(copies) : 1;
  const book = { id: books.reduce((m, b) => Math.max(m, b.id), 0) + 1, title, author, copies: n, available: n };
  books.push(book);
  res.status(201).json(book);
});

module.exports = router;
module.exports.books = books;
