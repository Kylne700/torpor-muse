# notes

A small notes service. Node only, no dependencies, data in JSON files.

    PORT=3000 node server.js

Demo accounts: `alice` / `alice-pass-1` and `bob` / `bob-pass-2`.

- `POST /register`, `POST /login`, `POST /logout`, `GET /me`
- `GET /notes`, `POST /notes`, `GET /notes/:id`, `DELETE /notes/:id`

Everything under `/notes` needs a login (a session cookie). Users are in `data/users.json`,
notes in `data/notes.json`.
