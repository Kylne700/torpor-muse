# muse-sandbox

Runs a command in a bubblewrap sandbox that never shares the host network. `--net meta` lets traffic out only to an allowlist (default `api.meta.ai:443`) through a host-side proxy that logs every connection.

    scripts/sandbox/muse-sandbox.sh --run-dir DIR [--ro PATH]... [--net none|meta] [--allow HOST:PORT]... [--chdir DIR2] [--drain SECS] -- CMD [ARGS...]

Exit code is CMD's own; 125 means refused (bad option, `--run-dir` missing, relative, `/`, or the home or above it, no bwrap). Needs bwrap; `--net meta` also needs node 20+. Pass the Muse install folder with `--ro`; a key comes only from the caller's env (`META_API_KEY=... muse-sandbox.sh ...`), never argv.

**Isolated:** network (own namespace: no host loopback, no X display, no DNS), ipc, pids, the real home (empty tmpfs; only DIR, `DIR/muse-share` as `~/.local/share/muse` and `--ro` paths are bound), `/tmp`, `/run`, the environment (cleared; only `TORPOR_*` and, with `--net meta`, `META_API_KEY` pass) and `DIR/net` (proxy log and socket). `/` is read-only.
**Not isolated:** `/` stays readable, `/proc` and `/sys` are visible, a kernel bug defeats it, and an allowed host sees everything sent to it. `TERM` is not passed (use `env TERM=... CMD`).

**Mechanism: transparent forwarding, plus `HTTPS_PROXY` for clients that honour it.** Muse gives hooks (and what they start) only HOME LANG MUSE_SESSION_ID MUSE_THREAD_ID PATH PWD TMPDIR TRACEPARENT (selftest check 3 prints them), so a proxy variable cannot reach them. Instead each allowed name maps to `127.0.1.N` in the sandbox's `/etc/hosts`; a forwarder there relays raw bytes over a unix socket to the host proxy, which checks HOST:PORT. TLS is never opened. Two bwrap layers: one bwrap cannot bind :443 in its network namespace, so the outer holds the namespace and listeners and the inner holds the command (no capabilities).

**Log** `DIR/net/proxy.log`: `<UTC time> host:port allowed|denied up=N down=N via=fwd|proxy [err=CODE]`. Only connections that reach the proxy or a forwarder are logged; a client that ignores the proxy and asks for an unlisted name fails at DNS with no line. IP-literal allow entries (not 127.x) work only for `HTTPS_PROXY` clients.

**Differences from the first design:** `--unshare-pid` is on, so a model-run command cannot signal your processes. Because the detached writer must outlive Muse, `--net meta` waits up to `--drain` seconds (default 90, `0` = off) for leftover processes before closing the proxy. `/run` is a tmpfs, not only `/run/user`, which hides the `/run/dbus` and `/run/libvirt` sockets.

**Finding (one keyed `exec` ping, `muse-spark-1.3-contributor`):** Muse contacted only `api.meta.ai:443` (4 connections, all `via=proxy`), so the default allowlist stays. After keyed runs delete `tbh-linux-helper-payload-*` in the sandbox HOME tmp folder and in `DIR/tmp` (the build brief says they may hold the key; not checked here).

**Self-test** (offline, no key, no model call, about 5 s; never in the gate or CI): `MUSE_BIN=/path/to/muse scripts/sandbox/selftest.sh` prints PASS or FAIL per assert and exits 0 only if all pass.
