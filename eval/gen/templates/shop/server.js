'use strict';
// shop: a small shop API. Run it with: PORT=3000 node server.js
const http = require('http');
const catalog = require('./lib/catalog');
const carts = require('./lib/carts');
const orders = require('./lib/orders');

const PORT = Number(process.env.PORT) || 3000;
//#if R_coupon
const COUPONS = { WELCOME10: 10 };
//#endif

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

//#if R_customer
function customerOf(req) {
  const name = String(req.headers['x-customer'] || '').trim();
  if (!name) throw Object.assign(new Error('X-Customer header required'), { status: 400 });
  return name;
}
//#else
function customerOf(req) {
  return String(req.headers['x-customer'] || '').trim() || 'guest';
}
//#endif

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
//#if R_coupon
  const subtotalCents = items.reduce((sum, item) => sum + item.lineCents, 0);
  const coupon = carts.couponOf(customer);
  const discountCents = coupon ? Math.floor((subtotalCents * coupon.percent) / 100) : 0;
  const totalCents = subtotalCents - discountCents;
//#else
//#if R_vat=slip
  const subtotalCents = items.reduce((sum, item) => sum + item.lineCents, 0);
  const vatCents = Math.round((subtotalCents * 17) / 117);
  const totalCents = subtotalCents + vatCents;
//#else
  const totalCents = items.reduce((sum, item) => sum + item.lineCents, 0);
//#endif
//#endif
//#if R_vat=ref
  const vatCents = Math.round((totalCents * 17) / 117);
//#endif
  return {
    items,
//#if R_coupon
    discountCents,
//#endif
//#if R_vat
    vatCents,
//#endif
    totalCents,
  };
}

function home({ res }) {
  send(res, 200, PAGE);
}

//#if S4
function health({ res }) {
  send(res, 200, { status: 'ok' });
}

//#endif
//#if R_customer=slip
function listProducts({ req, res, url }) {
  customerOf(req);
//#else
function listProducts({ res, url }) {
//#endif
  let rows = catalog.all();
//#if N_search
  const q = (url.searchParams.get('q') || '').toLowerCase();
  if (q) rows = rows.filter((p) => p.name.toLowerCase().includes(q));
//#endif
  send(res, 200, rows);
}

//#if E_product_id
//#if R_customer=slip
function getProduct({ req, res, params }) {
  customerOf(req);
//#else
function getProduct({ res, params }) {
//#endif
  const product = catalog.byId(Number(params[0]));
  if (!product) return send(res, 404, { error: 'no such product' });
  send(res, 200, product);
}

//#endif
//#if S6
function categories({ res }) {
  send(res, 200, [...new Set(catalog.all().map((p) => p.category))].sort());
}

//#endif
function showCart({ req, res }) {
  send(res, 200, cartView(customerOf(req)));
}

//#if S5
function cartCount({ req, res }) {
  const count = carts.linesOf(customerOf(req)).reduce((sum, line) => sum + line.qty, 0);
  send(res, 200, { count });
}

//#endif
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

//#if R_coupon
async function applyCoupon({ req, res }) {
  const customer = customerOf(req);
  const { code } = await readJson(req);
  const percent = COUPONS[code];
  if (typeof percent !== 'number') return send(res, 400, { error: 'unknown coupon' });
  carts.setCoupon(customer, { code, percent });
  send(res, 200, cartView(customer));
}

//#endif
function checkout({ req, res }) {
  const customer = customerOf(req);
  const view = cartView(customer);
  if (!view.items.length) return send(res, 400, { error: 'the cart is empty' });
//#if R_coupon=slip
  const subtotalCents = view.items.reduce((sum, item) => sum + item.lineCents, 0);
  const discountCents = Math.floor((subtotalCents * carts.couponOf(customer).percent) / 100);
//#endif
  const order = orders.create(customer, {
    items: view.items,
//#if R_coupon=ref
    discountCents: view.discountCents,
//#endif
//#if R_coupon=slip
    discountCents,
//#endif
//#if R_vat
    vatCents: view.vatCents,
//#endif
//#if R_coupon=slip
    totalCents: subtotalCents - discountCents,
//#else
    totalCents: view.totalCents,
//#endif
  });
//#if E_clear
  carts.clear(customer);
//#endif
  send(res, 201, order);
}

function listOrders({ req, res }) {
  send(res, 200, orders.ordersOf(customerOf(req)));
}

//#if N_order
function getOrder({ req, res, params }) {
  const order = orders.getOrder(customerOf(req), Number(params[0]));
  if (!order) return send(res, 404, { error: 'no such order' });
  send(res, 200, order);
}

//#endif
async function dispatch(req, res) {
  const url = new URL(req.url, 'http://localhost');
  const { pathname } = url;
  const method = req.method;
  const ctx = { req, res, url, params: [] };

  if (method === 'GET' && pathname === '/') return home(ctx);
//#if S4
  if (method === 'GET' && pathname === '/health') return health(ctx);
//#endif
  if (method === 'GET' && pathname === '/products') return listProducts(ctx);
//#if E_product_id
  const product = /^\/products\/(\d+)$/.exec(pathname);
  if (method === 'GET' && product) return getProduct({ ...ctx, params: [product[1]] });
//#endif
//#if S6
  if (method === 'GET' && pathname === '/categories') return categories(ctx);
//#endif
  if (method === 'GET' && pathname === '/cart') return showCart(ctx);
//#if S5
  if (method === 'GET' && pathname === '/cart/count') return cartCount(ctx);
//#endif
//#if R_coupon
  if (method === 'POST' && pathname === '/cart/coupon') return applyCoupon(ctx);
//#endif
  if (method === 'POST' && pathname === '/cart/items') return addItem(ctx);
  const line = /^\/cart\/items\/(\d+)$/.exec(pathname);
  if (method === 'DELETE' && line) return removeItem({ ...ctx, params: [line[1]] });
  if (method === 'POST' && pathname === '/checkout') return checkout(ctx);
  if (method === 'GET' && pathname === '/orders') return listOrders(ctx);
//#if N_order
  const order = /^\/orders\/(\d+)$/.exec(pathname);
  if (method === 'GET' && order) return getOrder({ ...ctx, params: [order[1]] });
//#endif
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
