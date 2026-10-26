# todo

A small todo API. Everything is JSON except `GET /`.

    PORT=3000 node server.js

Demo accounts: `alice` / `alice-pass-1` and `bob` / `bob-pass-2`.
Get a token with `POST /token` and send it as `Authorization: Bearer <token>`.
Data lives in `data/db.json`.
