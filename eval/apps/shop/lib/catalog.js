'use strict';
const fs = require('fs');
const path = require('path');

const products = JSON.parse(fs.readFileSync(path.join(__dirname, '..', 'data', 'products.json'), 'utf8'));

const all = () => products;
const byId = (id) => products.find((p) => p.id === id);

module.exports = { all, byId };
