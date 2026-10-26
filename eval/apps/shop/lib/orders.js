'use strict';
const fs = require('fs');
const path = require('path');

const FILE = path.join(__dirname, '..', 'data', 'orders.json');

let orders = [];
try {
  orders = JSON.parse(fs.readFileSync(FILE, 'utf8'));
} catch (err) {
  if (err.code !== 'ENOENT') throw err;
}

function save() {
  fs.writeFileSync(FILE, JSON.stringify(orders, null, 2) + '\n');
}

const ordersOf = (customer) => orders.filter((order) => order.customer === customer);

function create(customer, fields) {
  const id = orders.reduce((max, order) => Math.max(max, order.id), 0) + 1;
  const order = { id, customer, ...fields, placedAt: new Date().toISOString() };
  orders.push(order);
  save();
  return order;
}

module.exports = { ordersOf, create };
