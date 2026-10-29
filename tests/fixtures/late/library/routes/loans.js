'use strict';
const express = require('express');
const { books } = require('./books');

const router = express.Router();
const loans = [];
const readers = new Map();

const LOAN_DAYS = 21;
const MAX_LOANS_PER_READER = 5;
const DAY_MS = 24 * 60 * 60 * 1000;

function readerOf(name) {
  const key = String(name || '').trim().toLowerCase();
  if (!readers.has(key)) readers.set(key, { name: String(name || '').trim(), loans: 0 });
  return readers.get(key);
}

router.get('/', (req, res) => {
  let rows = loans;
  if (req.query.reader) {
    const key = String(req.query.reader).toLowerCase();
    rows = rows.filter((l) => l.reader.toLowerCase() === key);
  }
  if (req.query.open === '1') rows = rows.filter((l) => !l.returnedAt);
  res.json({ loans: rows, count: rows.length });
});

router.get('/:id', (req, res) => {
  const loan = loans.find((l) => l.id === Number(req.params.id));
  if (!loan) return res.status(404).json({ error: 'loan not found' });
  res.json(loan);
});

// A reader borrows a book for three weeks. A reader can hold five books at the same time.
router.post('/', (req, res) => {
  const { bookId, reader } = req.body || {};
  const book = books.find((b) => b.id === Number(bookId));
  if (!book) return res.status(404).json({ error: 'book not found' });
  if (!reader) return res.status(400).json({ error: 'reader is required' });
  if (book.available < 1) return res.status(409).json({ error: 'no copy is available' });
  const who = readerOf(reader);
  if (who.loans >= MAX_LOANS_PER_READER) {
    return res.status(409).json({ error: 'this reader already holds ' + MAX_LOANS_PER_READER + ' books' });
  }
  const now = Date.now();
  const loan = { id: loans.length + 1, bookId: book.id, reader: who.name, borrowedAt: now, dueAt: now + LOAN_DAYS * DAY_MS, returnedAt: null };
  loans.push(loan);
  book.available -= 1;
  who.loans += 1;
  res.status(201).json(loan);
});

router.post('/:id/return', (req, res) => {
  const loan = loans.find((l) => l.id === Number(req.params.id));
  if (!loan) return res.status(404).json({ error: 'loan not found' });
  if (loan.returnedAt) return res.status(409).json({ error: 'this loan is closed' });
  loan.returnedAt = Date.now();
  const book = books.find((b) => b.id === loan.bookId);
  if (book) book.available += 1;
  readerOf(loan.reader).loans -= 1;
  res.json(loan);
});

// A reader can renew a loan once, for another LOAN_DAYS days, unless another reader is waiting for the book.
router.post('/:id/renew', (req, res) => {
  const loan = loans.find((l) => l.id === Number(req.params.id));
  if (!loan) return res.status(404).json({ error: 'loan not found' });
  if (loan.returnedAt) return res.status(409).json({ error: 'this loan is closed' });
  if (loan.renewed) return res.status(409).json({ error: 'this loan was already renewed' });
  if (holds.some((h) => h.bookId === loan.bookId)) return res.status(409).json({ error: 'someone is waiting for this book' });
  loan.dueAt += LOAN_DAYS * DAY_MS;
  loan.renewed = true;
  res.json(loan);
});

const holds = [];

// A reader joins the queue for a book that has no free copy.
router.post('/holds', (req, res) => {
  const { bookId, reader } = req.body || {};
  const book = books.find((b) => b.id === Number(bookId));
  if (!book) return res.status(404).json({ error: 'book not found' });
  if (book.available > 0) return res.status(409).json({ error: 'a copy is free, borrow it' });
  if (holds.some((h) => h.bookId === book.id && h.reader === reader)) {
    return res.status(409).json({ error: 'already in the queue' });
  }
  holds.push({ bookId: book.id, reader, since: Date.now() });
  res.status(201).json({ position: holds.filter((h) => h.bookId === book.id).length });
});

router.get('/holds/:bookId', (req, res) => {
  res.json({ queue: holds.filter((h) => h.bookId === Number(req.params.bookId)) });
});

// Everything one reader ever borrowed, newest first, with the days each book was kept.
router.get('/history/:reader', (req, res) => {
  const key = String(req.params.reader).toLowerCase();
  const rows = loans
    .filter((l) => l.reader.toLowerCase() === key)
    .map((l) => ({
      id: l.id,
      bookId: l.bookId,
      borrowedAt: l.borrowedAt,
      keptDays: Math.ceil(((l.returnedAt || Date.now()) - l.borrowedAt) / DAY_MS),
    }))
    .sort((a, b) => b.borrowedAt - a.borrowedAt);
  res.json({ reader: req.params.reader, loans: rows });
});

router.get('/popular/books', (req, res) => {
  const counts = new Map();
  for (const l of loans) counts.set(l.bookId, (counts.get(l.bookId) || 0) + 1);
  const top = [...counts.entries()].sort((a, b) => b[1] - a[1]).slice(0, 5);
  res.json({ books: top.map(([bookId, count]) => ({ bookId, count, title: (books.find((b) => b.id === bookId) || {}).title })) });
});

router.post('/:id/lost', (req, res) => {
  const loan = loans.find((l) => l.id === Number(req.params.id));
  if (!loan) return res.status(404).json({ error: 'loan not found' });
  loan.returnedAt = Date.now();
  loan.lost = true;
  const book = books.find((b) => b.id === loan.bookId);
  if (book) book.copies -= 1;
  readerOf(loan.reader).loans -= 1;
  res.json(loan);
});

router.get('/readers/:name', (req, res) => {
  const who = readers.get(String(req.params.name).toLowerCase());
  if (!who) return res.status(404).json({ error: 'reader not found' });
  res.json({ name: who.name, holding: who.loans, limit: MAX_LOANS_PER_READER });
});

router.get('/stats/open', (req, res) => {
  const open = loans.filter((l) => !l.returnedAt);
  const overdue = open.filter((l) => l.dueAt < Date.now());
  res.json({ open: open.length, overdue: overdue.length });
});

router.get('/due/soon', (req, res) => {
  const days = Number(req.query.days) || 3;
  const limit = Date.now() + days * DAY_MS;
  res.json({ loans: loans.filter((l) => !l.returnedAt && l.dueAt <= limit) });
});

// Totals for the desk: every closed or open loan that is past its due date, with what is owed so far.
router.get('/fines/summary', (req, res) => {
  const rows = loans
    .map((l) => ({ id: l.id, reader: l.reader, owed: lateFee(l, l.returnedAt ? new Date(l.returnedAt) : new Date()) }))
    .filter((r) => r.owed > 0);
  res.json({ fines: rows, total: rows.reduce((sum, r) => sum + r.owed, 0) });
});

const FEE_PER_DAY = 0.5;
const GRACE_DAYS = 1;
const MAX_FEE = 25;

// What a reader owes for a book that came back late: a fee for every day after the grace period, capped.
function lateFee(loan, today = new Date()) {
  const days = Math.floor((today - new Date(loan.dueAt)) / DAY_MS) - GRACE_DAYS;
  if (days <= 0) return 0;
  return Math.min(MAX_FEE, Math.round(days * FEE_PER_DAY * 100) / 100);
}

router.get('/:id/fee', (req, res) => {
  const loan = loans.find((l) => l.id === Number(req.params.id));
  if (!loan) return res.status(404).json({ error: 'loan not found' });
  const until = loan.returnedAt ? new Date(loan.returnedAt) : new Date();
  res.json({ id: loan.id, fee: lateFee(loan, until) });
});

router.get('/overdue/list', (req, res) => {
  const rows = loans.filter((l) => !l.returnedAt && l.dueAt < Date.now());
  res.json({ loans: rows.sort((a, b) => a.dueAt - b.dueAt), count: rows.length });
});

module.exports = router;
