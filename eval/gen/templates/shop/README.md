# shop

A small shop API. Everything is JSON except `GET /`.

    PORT=3000 node server.js

The catalogue is in `data/products.json` (prices are whole cents).
Say who you are with the `X-Customer` header; without it you are `guest`.
Carts live in memory and are gone after a restart. Orders are saved in
`data/orders.json`. Demo customers with old orders: `alice` and `bob`.
