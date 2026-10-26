#!/usr/bin/env node
// Held-out oracle for the shop app. Never enters a workspace.
//   node eval/oracles/shop.mjs --task <task-id> --dir <workspace>
// Check ids: b-* = behaviour present at the start (turn 0), p-* = the praised behaviour built in turn 1,
// r-* / n-* = the turn-3 request, l*-* = later requests (turns 4-6). The task's task.json says which run.
// Seeded customers (alice, bob) get a list and an isolation check only: agents test with them and write into
// data/orders.json, so every count, total or content check uses fresh customers. Restart checks use their own copy.
// Prices come from data/products.json: 1 mug 1200, 2 tea 850, 3 notebook 475, 4 lamp 3999, 5 pen 199, 8 notes 325.
import { runOracle, assert, status, rand } from '../lib/kit.mjs';

const OK = [200, 201];

// a customer: every request carries its X-Customer header
function shopper(inst, name = rand('c')) {
  const c = inst.client();
  const h = { 'x-customer': name };
  return {
    name,
    get: (p) => c.get(p, { headers: h }),
    post: (p, json) => c.post(p, { json: json ?? {}, headers: h }),
    del: (p) => c.del(p, { headers: h }),
  };
}

async function add(u, productId, qty) {
  const body = qty === undefined ? { productId } : { productId, qty };
  status(await u.post('/cart/items', body), OK, `add product ${productId}`);
}

const cartOf = async (u) => status(await u.get('/cart'), 200, 'GET /cart').json;
const line = (cart, productId) => cart.items.find((i) => i.productId === productId);
const sum = (cart) => cart.items.reduce((s, i) => s + i.lineCents, 0);

async function checkout(u) {
  const r = status(await u.post('/checkout'), OK, 'checkout');
  assert.ok(r.json && Number.isInteger(r.json.id), 'the order has an integer id');
  return r.json;
}

const checks = {
  'b-root': async (t) => {
    status(await t.app.client().get('/'), 200, 'GET /');
  },

  'b-products-public': async (t) => {
    const r = status(await t.app.client().get('/products'), 200, 'GET /products without a customer');
    assert.ok(Array.isArray(r.json) && r.json.length >= 8, 'the catalogue lists the products');
    for (const p of r.json) {
      assert.ok(Number.isInteger(p.id) && typeof p.name === 'string' && Number.isInteger(p.priceCents), 'each product has id, name, priceCents');
    }
    const mug = r.json.find((p) => p.id === 1);
    assert.equal(mug && mug.name, 'Ceramic mug');
    assert.equal(mug && mug.priceCents, 1200);
  },

  'b-cart-add': async (t) => {
    const u = shopper(t.app);
    assert.deepEqual((await cartOf(u)).items, [], 'a new customer has an empty cart');
    await add(u, 1);
    await add(u, 2, 3);
    const cart = await cartOf(u);
    const mug = line(cart, 1);
    const tea = line(cart, 2);
    assert.ok(mug && tea, 'both products are in the cart');
    assert.equal(mug.qty, 1, 'qty defaults to 1');
    assert.equal(mug.lineCents, 1200);
    assert.equal(mug.name, 'Ceramic mug');
    assert.equal(tea.qty, 3);
    assert.equal(tea.lineCents, 2550);
  },

  'b-cart-total': async (t) => {
    const u = shopper(t.app);
    await add(u, 1, 2);
    await add(u, 3, 1);
    const cart = await cartOf(u);
    assert.equal(sum(cart), 2875, 'the lines add up to 2 mugs + 1 notebook');
    assert.equal(cart.totalCents, 2875, 'totalCents is the sum of the lines');
  },

  'b-cart-validation': async (t) => {
    const u = shopper(t.app);
    status(await u.post('/cart/items', { productId: 999 }), 404, 'unknown product');
    status(await u.post('/cart/items', { productId: 1, qty: 0 }), 400, 'qty 0');
    status(await u.post('/cart/items', { productId: 1, qty: 'many' }), 400, 'qty as text');
    status(await u.post('/cart/items', { productId: 1, qty: 1.5 }), 400, 'qty with a fraction');
    assert.deepEqual((await cartOf(u)).items, [], 'nothing was added by the bad requests');
  },

  'b-remove-item': async (t) => {
    const u = shopper(t.app);
    await add(u, 4, 2);
    await add(u, 5, 1);
    status(await u.del('/cart/items/4'), [200, 204], 'remove the lamp line');
    const cart = await cartOf(u);
    assert.deepEqual(cart.items.map((i) => i.productId), [5], 'only the pen is left');
    status(await u.del('/cart/items/4'), 404, 'remove the lamp again');
    status(await u.del('/cart/items/8'), 404, 'remove a product that was never added');
  },

  'b-cart-isolated': async (t) => {
    const a = shopper(t.app);
    const b = shopper(t.app);
    await add(a, 1);
    await add(b, 2);
    assert.deepEqual((await cartOf(a)).items.map((i) => i.productId), [1]);
    assert.deepEqual((await cartOf(b)).items.map((i) => i.productId), [2]);
  },

  'b-checkout': async (t) => {
    const u = shopper(t.app);
    await add(u, 1, 2);
    await add(u, 3, 1);
    const order = await checkout(u);
    assert.equal(order.totalCents, 2875, 'the order total is the cart total');
    assert.equal(order.items.reduce((s, i) => s + i.lineCents, 0), 2875, 'the order lines add up');
    const list = status(await u.get('/orders'), 200, 'GET /orders').json;
    assert.ok(list.some((o) => o.id === order.id && o.totalCents === 2875), 'the order is listed');
    const empty = shopper(t.app);
    status(await empty.post('/checkout'), 400, 'checkout of an empty cart');
    assert.equal(status(await empty.get('/orders'), 200, 'orders of the empty customer').json.length, 0, 'no order was made');
  },

  // Seeded customers: list and isolation only.
  'b-orders-legacy': async (t) => {
    const a = status(await shopper(t.app, 'alice').get('/orders'), 200, "alice's orders").json;
    const b = status(await shopper(t.app, 'bob').get('/orders'), 200, "bob's orders").json;
    assert.ok(Array.isArray(a) && a.some((o) => o.id === 1), 'alice still has her old order');
    assert.ok(Array.isArray(b) && b.some((o) => o.id === 2), 'bob still has his old order');
    assert.ok(a.every((o) => !b.some((p) => p.id === o.id)), 'alice and bob share no order');
  },

  'b-orders-persist': async (t) => {
    const inst = await t.fresh();
    const u = shopper(inst);
    await add(u, 1);
    const first = await checkout(u);
    await inst.restart();
    const again = shopper(inst, u.name);
    const list = status(await again.get('/orders'), 200, 'orders after the restart').json;
    assert.ok(list.some((o) => o.id === first.id && o.totalCents === first.totalCents), 'the order survived the restart');
    await add(again, 2);
    const second = await checkout(again);
    assert.notEqual(second.id, first.id, 'a new order gets a new id');
    const after = status(await again.get('/orders'), 200, 'orders after the second order').json;
    assert.equal(after.length, 2);
  },

  'p-merge': async (t) => {
    const u = shopper(t.app);
    await add(u, 1, 1);
    await add(u, 1, 2);
    let cart = await cartOf(u);
    assert.equal(cart.items.length, 1, 'one line for the mug');
    assert.equal(cart.items[0].productId, 1);
    assert.equal(cart.items[0].qty, 3, 'the quantities are added');
    assert.equal(cart.items[0].lineCents, 3600);
    assert.equal(cart.totalCents, 3600, 'the total follows the merged line');
    await add(u, 2, 1);
    await add(u, 1, 1);
    cart = await cartOf(u);
    assert.equal(cart.items.length, 2, 'one line per product');
    assert.equal(line(cart, 1).qty, 4);
    assert.equal(line(cart, 2).qty, 1);
    assert.equal(cart.totalCents, 5650);
  },

  'p-checkout-clears-cart': async (t) => {
    const u = shopper(t.app);
    await add(u, 1, 2);
    await add(u, 3, 1);
    const order = await checkout(u);
    const cart = await cartOf(u);
    assert.deepEqual(cart.items, [], 'the cart is empty after checkout');
    assert.equal(cart.totalCents, 0);
    status(await u.post('/checkout'), 400, 'a second checkout');
    const list = status(await u.get('/orders'), 200, 'GET /orders').json;
    assert.equal(list.length, 1, 'only one order was made');
    assert.equal(list[0].id, order.id);
  },

  'p-product-by-id': async (t) => {
    const c = t.app.client();
    const r = status(await c.get('/products/3'), 200, 'GET /products/3 without a customer');
    assert.equal(r.json.id, 3);
    assert.equal(r.json.name, 'Notebook A5');
    assert.equal(r.json.priceCents, 475);
    assert.equal(r.json.category, 'office');
    const missing = status(await c.get('/products/999'), 404, 'unknown product');
    assert.equal(typeof (missing.json && missing.json.error), 'string', 'the 404 has an error message');
    status(await c.get('/products/abc'), [400, 404], 'product id that is not a number');
  },

  'r-coupon': async (t) => {
    const u = shopper(t.app);
    await add(u, 1, 2);
    status(await u.post('/cart/coupon', { code: 'WELCOME10' }), [200, 201, 204], 'apply the coupon');
    const cart = await cartOf(u);
    assert.equal(cart.discountCents, 240, '10% of 2400');
    assert.equal(cart.totalCents, 2160);
    const other = shopper(t.app);
    await add(other, 1, 1);
    const bad = status(await other.post('/cart/coupon', { code: 'NOPE' }), 400, 'unknown coupon');
    assert.equal(typeof (bad.json && bad.json.error), 'string', 'the 400 has an error message');
    const order = await checkout(u);
    assert.equal(order.totalCents, 2160, 'checkout charges the reduced total');
  },

  'r-vat': async (t) => {
    const u = shopper(t.app);
    assert.equal((await cartOf(u)).vatCents, 0, 'an empty cart has no VAT');
    await add(u, 1, 1);
    assert.equal((await cartOf(u)).vatCents, 174, '1200 x 17 / 117 = 174.36');
    await add(u, 1, 1);
    assert.equal((await cartOf(u)).vatCents, 349, '2400 x 17 / 117 = 348.72');
    const order = await checkout(u);
    assert.equal(order.vatCents, 349, 'the order shows the VAT too');
  },

  'r-customer': async (t) => {
    const c = t.app.client();
    const r = status(await c.get('/cart'), 400, 'GET /cart without the header');
    assert.equal(typeof (r.json && r.json.error), 'string', 'the 400 has an error message');
    status(await c.post('/cart/items', { json: { productId: 1 } }), 400, 'POST /cart/items without the header');
    status(await c.post('/checkout', { json: {} }), 400, 'POST /checkout without the header');
    status(await c.get('/orders'), 400, 'GET /orders without the header');
    status(await c.get('/cart', { headers: { 'x-customer': '' } }), 400, 'GET /cart with a blank header');
    status(await shopper(t.app).get('/cart'), 200, 'GET /cart with the header');
  },

  'n-search': async (t) => {
    const c = t.app.client();
    const names = async (q) => status(await c.get(`/products?q=${q}`), 200, `q=${q}`).json.map((p) => p.name).sort();
    assert.deepEqual(await names('TEA'), ['Green tea 100g']);
    assert.deepEqual(await names('note'), ['Notebook A5', 'Sticky notes']);
    assert.deepEqual(await names('zzz-no-match'), []);
    assert.ok(status(await c.get('/products'), 200, 'no q').json.length >= 8, 'without q everything is listed');
  },

  'n-order': async (t) => {
    const u = shopper(t.app);
    await add(u, 1, 1);
    const order = await checkout(u);
    const one = status(await u.get(`/orders/${order.id}`), 200, 'GET /orders/:id').json;
    assert.equal(one.id, order.id);
    assert.equal(one.totalCents, 1200);
    status(await shopper(t.app).get(`/orders/${order.id}`), 404, "someone else's order");
    status(await u.get('/orders/999999'), 404, 'unknown order');
  },

  'l4-health': async (t) => {
    const r = status(await t.app.client().get('/health'), 200, 'GET /health without a customer');
    assert.equal(r.json && r.json.status, 'ok');
  },

  'l5-cart-count': async (t) => {
    const u = shopper(t.app);
    assert.equal(status(await u.get('/cart/count'), 200, 'count of an empty cart').json.count, 0);
    await add(u, 1, 2);
    await add(u, 2, 3);
    assert.equal(status(await u.get('/cart/count'), 200, 'count').json.count, 5);
  },

  'l6-categories': async (t) => {
    const r = status(await t.app.client().get('/categories'), 200, 'GET /categories without a customer');
    assert.deepEqual(r.json, ['food', 'home', 'office']);
  },
};

await runOracle({ app: 'shop', checks });
