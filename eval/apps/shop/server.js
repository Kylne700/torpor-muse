'use strict';
// shop: a small shop API. Run it with: PORT=3000 node server.js
const http = require('http');
const catalog = require('./lib/catalog');
const carts = require('./lib/carts');
const orders = require('./lib/orders');

const PORT = Number(process.env.PORT) || 3000;

const PAGE = `<!doctype html>
<title>shop</title>
<h1>shop</h1>
<p>Browse /products, then fill your cart with POST /cart/items. Say who you are with the X-Customer header.</p>
`;

function send(res, status, body) {
  const isText = typeof body === 'string';
  res.writeHead(status, { 'Content-Type': isText ? 'text/html; charset=utf-8' : 'application/json' });
  res.end(isText ? body : JSON.stringify(body));
}

function readJson(req) {
  return new Promise((resolve, reject) => {
    let raw = '';
    req.on('data', (chunk) => (raw += chunk));
    req.on('end', () => {
      try {
        const body = raw ? JSON.parse(raw) : {};
        resolve(body && typeof body === 'object' ? body : {});
      } catch (err) {
        reject(Object.assign(new Error('invalid JSON'), { status: 400 }));
      }
    });
  });
}

function customerOf(req) {
  return String(req.headers['x-customer'] || '').trim() || 'guest';
}

function cartView(customer) {
  const items = carts.linesOf(customer).map((line) => {
    const product = catalog.byId(line.productId);
    return {
      productId: line.productId,
      name: product.name,
      qty: line.qty,
      priceCents: product.priceCents,
      lineCents: product.priceCents * line.qty,
    };
  });
  const totalCents = items.reduce((sum, item) => sum + item.lineCents, 0);
  return {
    items,
    totalCents,
  };
}

function home({ res }) {
  send(res, 200, PAGE);
}

function listProducts({ res, url }) {
  let rows = catalog.all();
  send(res, 200, rows);
}

function showCart({ req, res }) {
  send(res, 200, cartView(customerOf(req)));
}

async function addItem({ req, res }) {
  const customer = customerOf(req);
  const { productId, qty = 1 } = await readJson(req);
  if (!Number.isInteger(productId)) return send(res, 400, { error: 'productId must be a whole number' });
  if (!catalog.byId(productId)) return send(res, 404, { error: 'no such product' });
  if (!Number.isInteger(qty) || qty < 1 || qty > 99) {
    return send(res, 400, { error: 'qty must be a whole number from 1 to 99' });
  }
  carts.add(customer, productId, qty);
  send(res, 201, cartView(customer));
}

function removeItem({ req, res, params }) {
  const customer = customerOf(req);
  if (!carts.remove(customer, Number(params[0]))) return send(res, 404, { error: 'not in the cart' });
  send(res, 200, cartView(customer));
}

function checkout({ req, res }) {
  const customer = customerOf(req);
  const view = cartView(customer);
  if (!view.items.length) return send(res, 400, { error: 'the cart is empty' });
  const order = orders.create(customer, {
    items: view.items,
    totalCents: view.totalCents,
  });
  send(res, 201, order);
}

function listOrders({ req, res }) {
  send(res, 200, orders.ordersOf(customerOf(req)));
}

async function dispatch(req, res) {
  const url = new URL(req.url, 'http://localhost');
  const { pathname } = url;
  const method = req.method;
  const ctx = { req, res, url, params: [] };

  if (method === 'GET' && pathname === '/') return home(ctx);
  if (method === 'GET' && pathname === '/products') return listProducts(ctx);
  if (method === 'GET' && pathname === '/cart') return showCart(ctx);
  if (method === 'POST' && pathname === '/cart/items') return addItem(ctx);
  const line = /^\/cart\/items\/(\d+)$/.exec(pathname);
  if (method === 'DELETE' && line) return removeItem({ ...ctx, params: [line[1]] });
  if (method === 'POST' && pathname === '/checkout') return checkout(ctx);
  if (method === 'GET' && pathname === '/orders') return listOrders(ctx);
  send(res, 404, { error: 'not found' });
}

http
  .createServer((req, res) => {
    dispatch(req, res).catch((err) => {
      if (!err.status) console.error(err.stack);
      send(res, err.status || 500, { error: err.status ? err.message : 'server error' });
    });
  })
  .listen(PORT, '127.0.0.1', () => console.log(`shop listening on ${PORT}`));
