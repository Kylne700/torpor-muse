'use strict';
// A small blog: posts with comments, tags and authors, and an RSS feed.
const express = require('express');

const app = express();
app.use(express.json());

const PORT = Number(process.env.PORT) || 4000;
const PAGE_SIZE = 10;
const SITE_URL = 'https://blog.example.test';

const db = {
  posts: [
    { id: 1, slug: 'hello-world', title: 'Hello world', body: 'First post.', published: true, createdAt: 1700000000000 },
    { id: 2, slug: 'draft-ideas', title: 'Draft ideas', body: 'Not ready yet.', published: false, createdAt: 1700100000000 },
  ],
  comments: [],
  tags: [],
  authors: [],
};

function nextId(rows) {
  return rows.reduce((max, r) => Math.max(max, r.id), 0) + 1;
}

function slugify(title) {
  return String(title).toLowerCase().replace(/[^a-z0-9]+/g, '-').replace(/^-|-$/g, '');
}

// Published posts, newest first. Drafts only show up with ?drafts=1.
app.get('/api/posts', (req, res) => {
  const all = req.query.drafts === '1';
  const rows = db.posts.filter((p) => all || p.published).sort((a, b) => b.createdAt - a.createdAt);
  res.json({ posts: rows.slice(0, PAGE_SIZE), count: rows.length });
});

app.post('/api/posts', (req, res) => {
  const { title, body, published } = req.body || {};
  if (!title || !body) return res.status(400).json({ error: 'a post needs a title and a body' });
  const slug = slugify(title);
  if (db.posts.some((p) => p.slug === slug)) {
    return res.status(409).json({ error: 'a post with this title already exists' });
  }
  const post = { id: nextId(db.posts), slug, title, body, published: Boolean(published), createdAt: Date.now() };
  db.posts.push(post);
  res.status(201).json(post);
});

app.get('/api/posts/:slug', (req, res) => {
  const post = db.posts.find((p) => p.slug === req.params.slug);
  if (!post || !post.published) return res.status(404).json({ error: 'post not found' });
  res.json({ ...post, comments: db.comments.filter((c) => c.postId === post.id) });
});

app.put('/api/posts/:slug', (req, res) => {
  const post = db.posts.find((p) => p.slug === req.params.slug);
  if (!post) return res.status(404).json({ error: 'post not found' });
  const { title, body, published } = req.body || {};
  if (title) post.title = title;
  if (body) post.body = body;
  if (published !== undefined) post.published = Boolean(published);
  res.json(post);
});

// ---- tags ----
app.get('/api/tags', (req, res) => {
  const page = Math.max(1, Number(req.query.page) || 1);
  let rows = db.tags;
  res.json({ tags: rows.slice((page - 1) * PAGE_SIZE, page * PAGE_SIZE), page, count: rows.length });
});

app.get('/api/tags/:id', (req, res) => {
  const tag = db.tags.find((r) => r.id === Number(req.params.id));
  if (!tag) return res.status(404).json({ error: 'tag not found' });
  res.json(tag);
});

app.post('/api/tags', (req, res) => {
  const body = req.body || {};
  if (!body.name) return res.status(400).json({ error: 'name is required' });
  const tag = { id: nextId(db.tags), name: body.name, createdAt: Date.now() };
  db.tags.push(tag);
  res.status(201).json(tag);
});

app.put('/api/tags/:id', (req, res) => {
  const tag = db.tags.find((r) => r.id === Number(req.params.id));
  if (!tag) return res.status(404).json({ error: 'tag not found' });
  Object.assign(tag, req.body || {}, { id: tag.id });
  res.json(tag);
});

app.delete('/api/tags/:id', (req, res) => {
  const at = db.tags.findIndex((r) => r.id === Number(req.params.id));
  if (at < 0) return res.status(404).json({ error: 'tag not found' });
  db.tags.splice(at, 1);
  res.status(204).end();
});

// ---- authors ----
app.get('/api/authors', (req, res) => {
  const page = Math.max(1, Number(req.query.page) || 1);
  let rows = db.authors;
  res.json({ authors: rows.slice((page - 1) * PAGE_SIZE, page * PAGE_SIZE), page, count: rows.length });
});

app.get('/api/authors/:id', (req, res) => {
  const author = db.authors.find((r) => r.id === Number(req.params.id));
  if (!author) return res.status(404).json({ error: 'author not found' });
  res.json(author);
});

app.post('/api/authors', (req, res) => {
  const body = req.body || {};
  if (!body.name) return res.status(400).json({ error: 'name is required' });
  if (!body.bio) return res.status(400).json({ error: 'bio is required' });
  const author = { id: nextId(db.authors), name: body.name, bio: body.bio, createdAt: Date.now() };
  db.authors.push(author);
  res.status(201).json(author);
});

app.put('/api/authors/:id', (req, res) => {
  const author = db.authors.find((r) => r.id === Number(req.params.id));
  if (!author) return res.status(404).json({ error: 'author not found' });
  Object.assign(author, req.body || {}, { id: author.id });
  res.json(author);
});

app.delete('/api/authors/:id', (req, res) => {
  const at = db.authors.findIndex((r) => r.id === Number(req.params.id));
  if (at < 0) return res.status(404).json({ error: 'author not found' });
  db.authors.splice(at, 1);
  res.status(204).end();
});

app.get('/api/posts/:slug/comments', (req, res) => {
  const post = db.posts.find((p) => p.slug === req.params.slug);
  if (!post) return res.status(404).json({ error: 'post not found' });
  res.json({ comments: db.comments.filter((c) => c.postId === post.id) });
});

app.post('/api/posts/:slug/comments', (req, res) => {
  const post = db.posts.find((p) => p.slug === req.params.slug);
  if (!post) return res.status(404).json({ error: 'post not found' });
  const { name, text } = req.body || {};
  if (!name || !text) return res.status(400).json({ error: 'name and text are required' });
  const comment = { id: nextId(db.comments), postId: post.id, name, text, createdAt: Date.now() };
  db.comments.push(comment);
  res.status(201).json(comment);
});

// ---- pages ----
app.get('/api/pages', (req, res) => {
  const page = Math.max(1, Number(req.query.page) || 1);
  let rows = db.pages;
  if (req.query.title) {
    const q = String(req.query.title).toLowerCase();
    rows = rows.filter((r) => String(r.title).toLowerCase().includes(q));
  }
  res.json({ pages: rows.slice((page - 1) * PAGE_SIZE, page * PAGE_SIZE), page, count: rows.length });
});

app.get('/api/pages/:id', (req, res) => {
  const page = db.pages.find((r) => r.id === Number(req.params.id));
  if (!page) return res.status(404).json({ error: 'page not found' });
  res.json(page);
});

app.post('/api/pages', (req, res) => {
  const body = req.body || {};
  if (!body.title) return res.status(400).json({ error: 'title is required' });
  if (!body.body) return res.status(400).json({ error: 'body is required' });
  const page = { id: nextId(db.pages), title: body.title, body: body.body, createdAt: Date.now() };
  db.pages.push(page);
  res.status(201).json(page);
});

app.put('/api/pages/:id', (req, res) => {
  const page = db.pages.find((r) => r.id === Number(req.params.id));
  if (!page) return res.status(404).json({ error: 'page not found' });
  Object.assign(page, req.body || {}, { id: page.id });
  res.json(page);
});

app.delete('/api/pages/:id', (req, res) => {
  const at = db.pages.findIndex((r) => r.id === Number(req.params.id));
  if (at < 0) return res.status(404).json({ error: 'page not found' });
  db.pages.splice(at, 1);
  res.status(204).end();
});

// Counts for the dashboard: posts per month and the number of comments waiting for a reply.
app.get('/api/stats', (req, res) => {
  const perMonth = {};
  for (const p of db.posts) {
    const month = new Date(p.createdAt).toISOString().slice(0, 7);
    perMonth[month] = (perMonth[month] || 0) + 1;
  }
  const unanswered = db.comments.filter((c) => !c.reply).length;
  res.json({ perMonth, unanswered, posts: db.posts.length });
});

// Removes one comment of a post (the blog owner cleans up spam this way).
app.delete('/api/posts/:slug/comments/:commentId', (req, res) => {
  const post = db.posts.find((p) => p.slug === req.params.slug);
  if (!post) return res.status(404).json({ error: 'post not found' });
  const at = db.comments.findIndex(
    (c) => c.id === Number(req.params.commentId) && c.postId === post.id,
  );
  if (at < 0) return res.status(404).json({ error: 'comment not found' });
  db.comments.splice(at, 1);
  res.status(204).end();
});

function escapeXml(s) {
  return String(s).replace(/&/g, '&amp;').replace(/</g, '&lt;').replace(/>/g, '&gt;');
}

// The RSS feed: the ten newest published posts, the way a feed reader expects them.
app.get('/feed.xml', (req, res) => {
  const posts = db.posts.filter((p) => p.published).sort((a, b) => b.createdAt - a.createdAt).slice(0, 10);
  const items = posts
    .map(
      (p) =>
        '<item><title>' + escapeXml(p.title) + '</title><link>' + SITE_URL + '/posts/' + p.slug + '</link>' +
        '<pubDate>' + new Date(p.createdAt).toUTCString() + '</pubDate></item>',
    )
    .join('');
  res
    .type('application/rss+xml')
    .send('<?xml version="1.0"?><rss version="2.0"><channel><title>Blog</title>' + items + '</channel></rss>');
});

app.use((req, res) => {
  res.status(404).json({ error: 'not found' });
});

app.listen(PORT, () => {
  console.log('blog listening on ' + PORT);
});
