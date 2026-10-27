#!/usr/bin/env python3
"""Hook runs recorded in Muse session logs. usage (python3 -I): hooks.py <sessions-root> [--last N] [--event E] [--times]
Prints one line per hook_run_terminal (and a line per SessionStart source) for the N newest session logs (default 1).
With --times every row starts with the log time (UTC, ms) and the started rows are printed too.
Fields: session short id, event, status, exit code, ms, error, stdout (cut), systemMessage (cut), effects."""
import datetime, glob, json, os, sys


def flat(r):
    yield r
    if isinstance(r, dict) and isinstance(r.get("children"), list):
        for c in r["children"]:
            try:
                yield json.loads(c["record_json"])
            except Exception:
                pass


def runs(path):
    for line in open(path, errors="replace"):
        try:
            r = json.loads(line)
        except Exception:
            continue
        for x in flat(r):
            pl = x.get("payload") if isinstance(x, dict) else None
            ev = pl.get("event") if isinstance(pl, dict) else None
            if isinstance(ev, dict) and ev.get("kind") in ("hook_run_started", "hook_run_terminal"):
                ev["_at"] = x.get("recorded_at") or r.get("recorded_at")
                yield ev


def cut(s, n):
    s = "" if s is None else str(s).replace("\n", "\\n")
    return s if len(s) <= n else s[:n] + "..."


if __name__ == "__main__":
    if len(sys.argv) < 2:
        sys.exit(__doc__)
    root = sys.argv[1]
    a = sys.argv[2:]
    last = int(a[a.index("--last") + 1]) if "--last" in a else 1
    only = a[a.index("--event") + 1] if "--event" in a else None
    times = "--times" in a
    logs = sorted(glob.glob(root + "/**/session.jsonl", recursive=True), key=os.path.getmtime)
    for p in logs[-last:]:
        sid = os.path.basename(os.path.dirname(p))
        print("== session", sid[:13], "(%d bytes)" % os.path.getsize(p))
        for ev in runs(p):
            if (ev["kind"] != "hook_run_terminal" and not times) or (only and ev.get("event") != only):
                continue
            t = ""
            if times:
                us = ev.get("_at")
                t = (datetime.datetime.fromtimestamp(us / 1e6, datetime.timezone.utc).strftime("%H:%M:%S.%f")[:-3] + " ") if isinstance(us, (int, float)) else "?            "
                if ev["kind"] == "hook_run_started":
                    print("%s%-16s started  %s" % (t, ev.get("event"), cut(ev.get("hook_key"), 90)))
                    continue
            print(t + "%-16s %-9s rc=%-4s %5sms err=%s out=%s sys=%s eff=%s" % (
                ev.get("event"), ev.get("status"), ev.get("exit_code"), ev.get("duration_ms"),
                cut(ev.get("error"), 160), cut(ev.get("stdout"), 160), cut(ev.get("system_message"), 100),
                cut(json.dumps(ev.get("effects")), 60)))
