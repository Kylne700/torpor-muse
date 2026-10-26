'use strict';
// Carts live in memory, one per customer, and are gone after a restart. Orders are the durable part.
const carts = new Map();
//#if R_coupon
const coupons = new Map();
//#endif

function linesOf(customer) {
  if (!carts.has(customer)) carts.set(customer, []);
  return carts.get(customer);
}

function findLine(customer, productId) {
//#if I_findline
  return linesOf(customer).find((line) => line.productId === String(productId));
//#else
  return linesOf(customer).find((line) => line.productId === productId);
//#endif
}

function add(customer, productId, qty) {
//#if E_merge
  const line = findLine(customer, productId);
  if (line) {
    line.qty += qty;
    return;
  }
//#endif
  linesOf(customer).push({ productId, qty });
}

function remove(customer, productId) {
  const line = findLine(customer, productId);
  if (!line) return false;
//#if I_remove
  if (line.qty > 1) {
    line.qty -= 1;
    return true;
  }
//#endif
  const lines = linesOf(customer);
  lines.splice(lines.indexOf(line), 1);
  return true;
}

function clear(customer) {
//#if I_cartclear
  linesOf(customer).slice(0);
//#else
  linesOf(customer).splice(0);
//#endif
//#if R_coupon
  coupons.delete(customer);
//#endif
}

//#if R_coupon
const setCoupon = (customer, coupon) => coupons.set(customer, coupon);
const couponOf = (customer) => coupons.get(customer);

//#endif
//#if R_coupon
module.exports = { linesOf, findLine, add, remove, clear, setCoupon, couponOf };
//#else
module.exports = { linesOf, findLine, add, remove, clear };
//#endif
