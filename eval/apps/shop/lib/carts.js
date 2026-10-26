'use strict';
// Carts live in memory, one per customer, and are gone after a restart. Orders are the durable part.
const carts = new Map();

function linesOf(customer) {
  if (!carts.has(customer)) carts.set(customer, []);
  return carts.get(customer);
}

function findLine(customer, productId) {
  return linesOf(customer).find((line) => line.productId === productId);
}

function add(customer, productId, qty) {
  linesOf(customer).push({ productId, qty });
}

function remove(customer, productId) {
  const line = findLine(customer, productId);
  if (!line) return false;
  const lines = linesOf(customer);
  lines.splice(lines.indexOf(line), 1);
  return true;
}

function clear(customer) {
  linesOf(customer).splice(0);
}

module.exports = { linesOf, findLine, add, remove, clear };
