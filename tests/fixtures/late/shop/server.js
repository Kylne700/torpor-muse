'use strict';
// Small shop API: products, a cart for each visitor, orders and a few admin reports.
const express = require('express');

const app = express();
app.use(express.json());

const PORT = Number(process.env.PORT) || 3000;
const PAGE_SIZE = 20;
// Prices are in shekels. Discounts are applied at the cart, never to the product price.
const VAT_PERCENT = 17;

const db = {
  products: [
    { id: 1, name: 'Ceramic mug', category: 'kitchen', price: 42, stock: 18 },
    { id: 2, name: 'Linen apron', category: 'kitchen', price: 89, stock: 7 },
    { id: 3, name: 'Desk lamp', category: 'office', price: 215, stock: 3 },
    { id: 4, name: 'Notebook set', category: 'office', price: 36, stock: 40 },
  ],
  categories: [{ id: 1, name: 'kitchen' }, { id: 2, name: 'office' }],
  customers: [],
  suppliers: [],
  reviews: [],
  orders: [],
};
const carts = new Map();

function nextId(rows) {
  return rows.reduce((max, r) => Math.max(max, r.id), 0) + 1;
}

function cartOf(req) {
  const id = req.get('x-visitor') || 'anonymous';
  if (!carts.has(id)) carts.set(id, { items: [], discount: null });
  return carts.get(id);
}

function money(n) {
  return Math.round(n * 100) / 100;
}

// Product list with a text search and a category filter.
app.get('/api/products', (req, res) => {
  const q = String(req.query.q || '').trim().toLowerCase();
  const category = req.query.category;
  let rows = db.products;
  if (q) rows = rows.filter((p) => p.name.toLowerCase().includes(q));
  if (category) rows = rows.filter((p) => p.category === category);
  res.json({ products: rows.slice(0, PAGE_SIZE), found: rows.length });
});

app.get('/api/products/:id', (req, res) => {
  const product = db.products.find((p) => p.id === Number(req.params.id));
  if (!product) return res.status(404).json({ error: 'product not found' });
  res.json(product);
});

app.post('/api/products', (req, res) => {
  const { name, category, price, stock } = req.body || {};
  if (!name || typeof price !== 'number') {
    return res.status(400).json({ error: 'name and a numeric price are required' });
  }
  const product = { id: nextId(db.products), name, category: category || 'misc', price, stock: stock || 0 };
  db.products.push(product);
  res.status(201).json(product);
});

// ---- categories ----
app.get('/api/categories', (req, res) => {
  const page = Math.max(1, Number(req.query.page) || 1);
  let rows = db.categories;
  res.json({ categories: rows.slice((page - 1) * PAGE_SIZE, page * PAGE_SIZE), page, count: rows.length });
});

app.get('/api/categories/:id', (req, res) => {
  const category = db.categories.find((r) => r.id === Number(req.params.id));
  if (!category) return res.status(404).json({ error: 'category not found' });
  res.json(category);
});

app.post('/api/categories', (req, res) => {
  const body = req.body || {};
  if (!body.name) return res.status(400).json({ error: 'name is required' });
  const category = { id: nextId(db.categories), name: body.name, createdAt: Date.now() };
  db.categories.push(category);
  res.status(201).json(category);
});

app.put('/api/categories/:id', (req, res) => {
  const category = db.categories.find((r) => r.id === Number(req.params.id));
  if (!category) return res.status(404).json({ error: 'category not found' });
  Object.assign(category, req.body || {}, { id: category.id });
  res.json(category);
});

app.delete('/api/categories/:id', (req, res) => {
  const at = db.categories.findIndex((r) => r.id === Number(req.params.id));
  if (at < 0) return res.status(404).json({ error: 'category not found' });
  db.categories.splice(at, 1);
  res.status(204).end();
});

// The home page shows the three newest products that are still in stock.
app.get('/api/featured', (req, res) => {
  const fresh = db.products.filter((p) => p.stock > 0).slice(-3).reverse();
  res.json({ featured: fresh });
});

app.get('/api/categories/:id/products', (req, res) => {
  const category = db.categories.find((c) => c.id === Number(req.params.id));
  if (!category) return res.status(404).json({ error: 'category not found' });
  res.json({ category: category.name, products: db.products.filter((p) => p.category === category.name) });
});

function cartTotal(cart) {
  return cart.items.reduce((sum, it) => sum + it.price * it.qty, 0);
}

// Puts a product in the visitor's cart; adding it again raises the quantity.
app.post('/api/cart/items', (req, res) => {
  const { productId, qty } = req.body || {};
  const product = db.products.find((p) => p.id === Number(productId));
  if (!product) return res.status(404).json({ error: 'no such product' });
  const cart = cartOf(req);
  const line = cart.items.find((it) => it.productId === product.id);
  if (line) line.qty += Number(qty) || 1;
  else cart.items.push({ productId: product.id, price: product.price, qty: Number(qty) || 1 });
  res.json({ items: cart.items, total: cartTotal(cart) });
});

app.get('/api/cart', (req, res) => {
  const cart = cartOf(req);
  res.json({ items: cart.items, discount: cart.discount, total: cartTotal(cart) });
});

app.delete('/api/cart/items/:productId', (req, res) => {
  const cart = cartOf(req);
  cart.items = cart.items.filter((it) => it.productId !== Number(req.params.productId));
  res.json({ items: cart.items, total: cartTotal(cart) });
});

// ---- customers ----
app.get('/api/customers', (req, res) => {
  const page = Math.max(1, Number(req.query.page) || 1);
  let rows = db.customers;
  if (req.query.name) {
    const q = String(req.query.name).toLowerCase();
    rows = rows.filter((r) => String(r.name).toLowerCase().includes(q));
  }
  res.json({ customers: rows.slice((page - 1) * PAGE_SIZE, page * PAGE_SIZE), page, count: rows.length });
});

app.get('/api/customers/:id', (req, res) => {
  const customer = db.customers.find((r) => r.id === Number(req.params.id));
  if (!customer) return res.status(404).json({ error: 'customer not found' });
  res.json(customer);
});

app.post('/api/customers', (req, res) => {
  const body = req.body || {};
  if (!body.name) return res.status(400).json({ error: 'name is required' });
  if (!body.email) return res.status(400).json({ error: 'email is required' });
  if (!body.phone) return res.status(400).json({ error: 'phone is required' });
  const customer = { id: nextId(db.customers), name: body.name, email: body.email, phone: body.phone, createdAt: Date.now() };
  db.customers.push(customer);
  res.status(201).json(customer);
});

app.put('/api/customers/:id', (req, res) => {
  const customer = db.customers.find((r) => r.id === Number(req.params.id));
  if (!customer) return res.status(404).json({ error: 'customer not found' });
  Object.assign(customer, req.body || {}, { id: customer.id });
  res.json(customer);
});

app.delete('/api/customers/:id', (req, res) => {
  const at = db.customers.findIndex((r) => r.id === Number(req.params.id));
  if (at < 0) return res.status(404).json({ error: 'customer not found' });
  db.customers.splice(at, 1);
  res.status(204).end();
});

const DISCOUNT_CODES = { WELCOME10: 10, SPRING15: 15, FRIEND5: 5 };

// Applies a discount code to the cart: a percentage off the whole cart, one code at a time.
app.post('/api/cart/discount', (req, res) => {
  const code = String((req.body || {}).code || '').trim().toUpperCase();
  const percent = DISCOUNT_CODES[code];
  if (!percent) return res.status(400).json({ error: 'unknown discount code' });
  const cart = cartOf(req);
  if (cart.discount) return res.status(409).json({ error: 'a discount code is already applied' });
  cart.discount = { code, percent };
  const total = cartTotal(cart);
  res.json({ code, percent, total, payable: money((total * (100 - percent)) / 100) });
});

// Turns the cart into an order. Stock is taken off the shelf here, not when the cart is filled.
app.post('/api/orders', (req, res) => {
  const cart = cartOf(req);
  if (cart.items.length === 0) return res.status(400).json({ error: 'the cart is empty' });
  for (const line of cart.items) {
    const product = db.products.find((p) => p.id === line.productId);
    if (!product || product.stock < line.qty) {
      return res.status(409).json({ error: 'not enough of product ' + line.productId });
    }
  }
  for (const line of cart.items) {
    db.products.find((p) => p.id === line.productId).stock -= line.qty;
  }
  const subtotal = cartTotal(cart);
  const off = cart.discount ? (subtotal * cart.discount.percent) / 100 : 0;
  const order = {
    id: nextId(db.orders),
    items: cart.items,
    status: 'new',
    total: money((subtotal - off) * (1 + VAT_PERCENT / 100)),
    createdAt: Date.now(),
  };
  db.orders.push(order);
  carts.delete(req.get('x-visitor') || 'anonymous');
  res.status(201).json(order);
});

// ---- suppliers ----
app.get('/api/suppliers', (req, res) => {
  const page = Math.max(1, Number(req.query.page) || 1);
  let rows = db.suppliers;
  res.json({ suppliers: rows.slice((page - 1) * PAGE_SIZE, page * PAGE_SIZE), page, count: rows.length });
});

app.get('/api/suppliers/:id', (req, res) => {
  const supplier = db.suppliers.find((r) => r.id === Number(req.params.id));
  if (!supplier) return res.status(404).json({ error: 'supplier not found' });
  res.json(supplier);
});

app.post('/api/suppliers', (req, res) => {
  const body = req.body || {};
  if (!body.name) return res.status(400).json({ error: 'name is required' });
  if (!body.city) return res.status(400).json({ error: 'city is required' });
  const supplier = { id: nextId(db.suppliers), name: body.name, city: body.city, createdAt: Date.now() };
  db.suppliers.push(supplier);
  res.status(201).json(supplier);
});

app.put('/api/suppliers/:id', (req, res) => {
  const supplier = db.suppliers.find((r) => r.id === Number(req.params.id));
  if (!supplier) return res.status(404).json({ error: 'supplier not found' });
  Object.assign(supplier, req.body || {}, { id: supplier.id });
  res.json(supplier);
});

app.delete('/api/suppliers/:id', (req, res) => {
  const at = db.suppliers.findIndex((r) => r.id === Number(req.params.id));
  if (at < 0) return res.status(404).json({ error: 'supplier not found' });
  db.suppliers.splice(at, 1);
  res.status(204).end();
});

// A customer can cancel an order until it is packed; the stock goes back on the shelf.
app.post('/api/orders/:id/cancel', (req, res) => {
  const order = db.orders.find((o) => o.id === Number(req.params.id));
  if (!order) return res.status(404).json({ error: 'order not found' });
  if (order.status !== 'new' && order.status !== 'paid') {
    return res.status(409).json({ error: 'this order can no longer be cancelled' });
  }
  for (const line of order.items) {
    const product = db.products.find((p) => p.id === line.productId);
    if (product) product.stock += line.qty;
  }
  order.status = 'cancelled';
  order.cancelledAt = Date.now();
  res.json({ id: order.id, status: order.status });
});

app.get('/api/orders', (req, res) => {
  const status = req.query.status;
  const rows = status ? db.orders.filter((o) => o.status === status) : db.orders;
  res.json({ orders: rows.slice(-PAGE_SIZE).reverse(), count: rows.length });
});

app.get('/api/orders/:id', (req, res) => {
  const order = db.orders.find((o) => o.id === Number(req.params.id));
  if (!order) return res.status(404).json({ error: 'order not found' });
  res.json(order);
});

app.post('/api/orders/:id/pack', (req, res) => {
  const order = db.orders.find((o) => o.id === Number(req.params.id));
  if (!order) return res.status(404).json({ error: 'order not found' });
  order.status = 'packed';
  res.json({ id: order.id, status: order.status });
});

// Sales per day for the admin page, newest day first.
app.get('/api/admin/sales', (req, res) => {
  const perDay = new Map();
  for (const o of db.orders) {
    if (o.status === 'cancelled') continue;
    const day = new Date(o.createdAt).toISOString().slice(0, 10);
    perDay.set(day, money((perDay.get(day) || 0) + o.total));
  }
  const days = [...perDay.entries()].sort((a, b) => (a[0] < b[0] ? 1 : -1));
  res.json({ days: days.map(([day, total]) => ({ day, total })) });
});

// ---- reviews ----
app.get('/api/reviews', (req, res) => {
  const page = Math.max(1, Number(req.query.page) || 1);
  let rows = db.reviews;
  res.json({ reviews: rows.slice((page - 1) * PAGE_SIZE, page * PAGE_SIZE), page, count: rows.length });
});

app.get('/api/reviews/:id', (req, res) => {
  const review = db.reviews.find((r) => r.id === Number(req.params.id));
  if (!review) return res.status(404).json({ error: 'review not found' });
  res.json(review);
});

app.post('/api/reviews', (req, res) => {
  const body = req.body || {};
  if (!body.productId) return res.status(400).json({ error: 'productId is required' });
  if (!body.stars) return res.status(400).json({ error: 'stars is required' });
  if (!body.text) return res.status(400).json({ error: 'text is required' });
  const review = { id: nextId(db.reviews), productId: body.productId, stars: body.stars, text: body.text, createdAt: Date.now() };
  db.reviews.push(review);
  res.status(201).json(review);
});

app.put('/api/reviews/:id', (req, res) => {
  const review = db.reviews.find((r) => r.id === Number(req.params.id));
  if (!review) return res.status(404).json({ error: 'review not found' });
  Object.assign(review, req.body || {}, { id: review.id });
  res.json(review);
});

app.delete('/api/reviews/:id', (req, res) => {
  const at = db.reviews.findIndex((r) => r.id === Number(req.params.id));
  if (at < 0) return res.status(404).json({ error: 'review not found' });
  db.reviews.splice(at, 1);
  res.status(204).end();
});

// ---- brands ----
app.get('/api/brands', (req, res) => {
  const page = Math.max(1, Number(req.query.page) || 1);
  let rows = db.brands;
  if (req.query.name) {
    const q = String(req.query.name).toLowerCase();
    rows = rows.filter((r) => String(r.name).toLowerCase().includes(q));
  }
  res.json({ brands: rows.slice((page - 1) * PAGE_SIZE, page * PAGE_SIZE), page, count: rows.length });
});

app.get('/api/brands/:id', (req, res) => {
  const brand = db.brands.find((r) => r.id === Number(req.params.id));
  if (!brand) return res.status(404).json({ error: 'brand not found' });
  res.json(brand);
});

app.post('/api/brands', (req, res) => {
  const body = req.body || {};
  if (!body.name) return res.status(400).json({ error: 'name is required' });
  if (!body.country) return res.status(400).json({ error: 'country is required' });
  const brand = { id: nextId(db.brands), name: body.name, country: body.country, createdAt: Date.now() };
  db.brands.push(brand);
  res.status(201).json(brand);
});

app.put('/api/brands/:id', (req, res) => {
  const brand = db.brands.find((r) => r.id === Number(req.params.id));
  if (!brand) return res.status(404).json({ error: 'brand not found' });
  Object.assign(brand, req.body || {}, { id: brand.id });
  res.json(brand);
});

app.delete('/api/brands/:id', (req, res) => {
  const at = db.brands.findIndex((r) => r.id === Number(req.params.id));
  if (at < 0) return res.status(404).json({ error: 'brand not found' });
  db.brands.splice(at, 1);
  res.status(204).end();
});

const LOW_STOCK_AT = 5;

function lowStockItems() {
  return db.products
    .filter((p) => p.stock <= LOW_STOCK_AT)
    .sort((a, b) => a.stock - b.stock)
    .map((p) => ({ id: p.id, name: p.name, stock: p.stock }));
}

app.get('/api/admin/low-stock', (req, res) => {
  const items = lowStockItems();
  res.json({ threshold: LOW_STOCK_AT, count: items.length, items });
});

app.get('/api/admin/orders.csv', (req, res) => {
  const lines = ['id,status,total,created'];
  for (const o of db.orders) {
    lines.push([o.id, o.status, o.total, new Date(o.createdAt).toISOString()].join(','));
  }
  res.type('text/csv').send(lines.join('\n') + '\n');
});

// ---- wishlists ----
app.get('/api/wishlists', (req, res) => {
  const page = Math.max(1, Number(req.query.page) || 1);
  let rows = db.wishlists;
  res.json({ wishlists: rows.slice((page - 1) * PAGE_SIZE, page * PAGE_SIZE), page, count: rows.length });
});

app.get('/api/wishlists/:id', (req, res) => {
  const wishlist = db.wishlists.find((r) => r.id === Number(req.params.id));
  if (!wishlist) return res.status(404).json({ error: 'wishlist not found' });
  res.json(wishlist);
});

app.post('/api/wishlists', (req, res) => {
  const body = req.body || {};
  if (!body.owner) return res.status(400).json({ error: 'owner is required' });
  if (!body.title) return res.status(400).json({ error: 'title is required' });
  const wishlist = { id: nextId(db.wishlists), owner: body.owner, title: body.title, createdAt: Date.now() };
  db.wishlists.push(wishlist);
  res.status(201).json(wishlist);
});

app.put('/api/wishlists/:id', (req, res) => {
  const wishlist = db.wishlists.find((r) => r.id === Number(req.params.id));
  if (!wishlist) return res.status(404).json({ error: 'wishlist not found' });
  Object.assign(wishlist, req.body || {}, { id: wishlist.id });
  res.json(wishlist);
});

app.delete('/api/wishlists/:id', (req, res) => {
  const at = db.wishlists.findIndex((r) => r.id === Number(req.params.id));
  if (at < 0) return res.status(404).json({ error: 'wishlist not found' });
  db.wishlists.splice(at, 1);
  res.status(204).end();
});

// ---- shipments ----
app.get('/api/shipments', (req, res) => {
  const page = Math.max(1, Number(req.query.page) || 1);
  let rows = db.shipments;
  res.json({ shipments: rows.slice((page - 1) * PAGE_SIZE, page * PAGE_SIZE), page, count: rows.length });
});

app.get('/api/shipments/:id', (req, res) => {
  const shipment = db.shipments.find((r) => r.id === Number(req.params.id));
  if (!shipment) return res.status(404).json({ error: 'shipment not found' });
  res.json(shipment);
});

app.post('/api/shipments', (req, res) => {
  const body = req.body || {};
  if (!body.orderId) return res.status(400).json({ error: 'orderId is required' });
  if (!body.carrier) return res.status(400).json({ error: 'carrier is required' });
  if (!body.city) return res.status(400).json({ error: 'city is required' });
  const shipment = { id: nextId(db.shipments), orderId: body.orderId, carrier: body.carrier, city: body.city, createdAt: Date.now() };
  db.shipments.push(shipment);
  res.status(201).json(shipment);
});

app.put('/api/shipments/:id', (req, res) => {
  const shipment = db.shipments.find((r) => r.id === Number(req.params.id));
  if (!shipment) return res.status(404).json({ error: 'shipment not found' });
  Object.assign(shipment, req.body || {}, { id: shipment.id });
  res.json(shipment);
});

app.delete('/api/shipments/:id', (req, res) => {
  const at = db.shipments.findIndex((r) => r.id === Number(req.params.id));
  if (at < 0) return res.status(404).json({ error: 'shipment not found' });
  db.shipments.splice(at, 1);
  res.status(204).end();
});

app.use((req, res) => {
  res.status(404).json({ error: 'not found' });
});

app.listen(PORT, () => {
  console.log(`shop listening on ${PORT}`);
});
