'use strict';
const express = require('express');
const loans = require('./routes/loans');
const books = require('./routes/books');

const app = express();
app.use(express.json());
app.use('/api/books', books);
app.use('/api/loans', loans);

app.use((req, res) => {
  res.status(404).json({ error: 'not found' });
});

const PORT = Number(process.env.PORT) || 8000;
app.listen(PORT, () => console.log('library listening on ' + PORT));
