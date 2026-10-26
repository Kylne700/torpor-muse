# Meta Model API mock

A zero-dependency Node (>= 20) server that imitates `POST /v1/responses` of the Meta Model API, failures included, so the check-writer is tested offline and in CI.

```
node mock/server.mjs --port <n|0> [--fixture a,b,c] [--output-text-file <path>] [--delay-ms <n>] [--log <file>]
```

- Binds 127.0.0.1 only. The first stdout line is `listening <port>` (use `--port 0` and read it). Bad arguments exit with code 2 and a message on stderr.
- `--fixture a,b,c`: request 1 gets `a`, request 2 gets `b`, and so on; the last one repeats. Default `ok`. Only `POST /v1/responses` is served and counted; any other method or path gets a 404 with an empty body, as the real API does for an unknown path (logged, not counted).
- `--output-text-file`: the text the model "wrote". It must exist at start and is read again on every request, as exact bytes (no trimming). `ok`, `slow` and `drop-mid-body` put it into the first `output_text` part. `truncated` adds a message item with `status: "incomplete"` and the first half of the text (cut by code points). Other fixtures ignore it.
- `--delay-ms`: how long `slow` waits (default 2000).
- `--log`: appended (created if missing). One JSON line per request, any path, written before the answer is sent: `{"method","path","headers","body"}`. Header names are lowercase. `body` is a JSON string holding the body as received, so parse each line with `JSON.parse` before searching it. Only `authorization` is redacted: `<redacted len=N>`, N being the length of the whole value including `Bearer `.
- Every response gets a fresh `id` (`resp_mock_<n>`) and `created_at` (and `completed_at` where the fixture has one). Everything else, `usage` too, is as recorded.
- Not covered: no 401 (the Authorization header is not checked), no request validation, no streaming.

## Fixtures

Each file in `fixtures/` has `_shape` (`recorded` or `assumed`), then `_model` and `_recorded_utc` (recorded) or `_source` (assumed), `_note`, `status`, `headers`, `body`. The test suite checks this table against the files.

| fixture | HTTP | shape | what it is |
|---|---|---|---|
| `ok` | 200 | recorded | completed; a reasoning item, then a message item with one `output_text` part; usage; `x-ratelimit-*` headers |
| `truncated` | 200 | recorded | `status: "incomplete"`, `incomplete_details.reason: "max_output_tokens"`, only the reasoning item (no message). With `--output-text-file` the added message item is assumed (copied from `ok`, status incomplete) |
| `refusal` | 200 | assumed | a message whose content part is `{"type":"refusal"}` (OpenAI convention; the Meta docs do not describe it) |
| `empty` | 200 | assumed | completed, only the reasoning item, no output text |
| `schema-400` | 400 | recorded | `invalid_request_error`, param `schema`, "Recursive JSON schemas are not currently supported" |
| `rate-429` | 429 | assumed | docs example, `Retry-After: 2` |
| `billing-402` | 402 | assumed | docs example, `billing_not_configured` |
| `server-500` | 500 | assumed | `server_error`; the message and code are placeholders |
| `slow` | 200 | ok envelope | the `ok` answer after `--delay-ms` |
| `drop-mid-body` | 200 | ok envelope | the full `Content-Length`, the first half of the bytes, then the socket is destroyed (the client sees ECONNRESET) |
| `hang` | none | not an envelope | reads the request, logs it, never answers |

Wrong-type and extra-key outputs are content: make them with `--output-text-file`.

## How the recorded ones were made

On 2026-10-26 (UTC), model `muse-spark-1.3`, reasoning effort low, a synthetic prompt, structured output via `text.format` of type `json_schema` (strict). Five real calls, about $0.014:

- `ok`: `max_output_tokens` 900. In the real answer the reasoning item comes first and the message second.
- `truncated`: budgets 470, 850 and 1020 all ended inside the reasoning item (reasoning used about 470 to 1020 tokens for a short answer), so a real partial message was never seen. The 850 one is the fixture.
- `schema-400`: a schema with a `$ref` to itself.

The rest follows https://dev.meta.ai/docs/error-handling, `/docs/protocols/responses` and `/docs/pricing-rate-limits` as read on 2026-10-26.

## Tests

`node --test mock/test/*.test.mjs` from the repo root. The bare `node --test mock/test/` fails with MODULE_NOT_FOUND on Node 22 and 24 (the argument is a file or glob there).
