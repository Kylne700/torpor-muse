const DEMO_EMAIL = "sam.demo@torpor";
const DEMO_PASSWORD = "demo-password-42";

const http = require("http");
const net = require("net");
const path = require("path");
const fs = require("fs/promises");
const { spawn } = require("child_process");

// The app under test always runs on loopback, so reach it directly
// instead of sending loopback traffic through any configured proxy.
for (const key of ["http_proxy", "https_proxy", "all_proxy", "HTTP_PROXY", "HTTPS_PROXY", "ALL_PROXY", "NODE_USE_ENV_PROXY"]) {
  delete process.env[key];
}

const USAGE = "Usage: node demo/pane.js DIR PORT";

function printUsageAndExit() {
  process.stderr.write(USAGE + "\n");
  process.exit(2);
}

const args = process.argv.slice(2);
if (args.length !== 2) {
  printUsageAndExit();
}
const appDir = args[0];
const portText = args[1];
if (!/^[0-9]+$/.test(portText)) {
  printUsageAndExit();
}
const panePort = Number(portText);
if (!Number.isInteger(panePort) || panePort < 1 || panePort > 65535) {
  printUsageAndExit();
}

const liveApps = new Set();

function stopTrackedAppsAndExit() {
  for (const child of liveApps) {
    try {
      child.kill("SIGKILL");
    } catch (err) {
    }
  }
  process.exit(0);
}

process.on("SIGTERM", stopTrackedAppsAndExit);
process.on("SIGINT", stopTrackedAppsAndExit);

function escapeHtml(value) {
  return String(value).replace(/[&<>"']/g, (ch) => {
    if (ch === "&") {
      return "&amp;";
    }
    if (ch === "<") {
      return "&lt;";
    }
    if (ch === ">") {
      return "&gt;";
    }
    if (ch === '"') {
      return "&quot;";
    }
    return "&#39;";
  });
}

function sleep(ms) {
  return new Promise((resolve) => {
    setTimeout(resolve, ms);
  });
}

async function fetchWithTimeout(url, options, ms) {
  const controller = new AbortController();
  const timer = setTimeout(() => {
    controller.abort();
  }, Math.max(1, ms));
  try {
    return await fetch(url, Object.assign({}, options, { signal: controller.signal }));
  } finally {
    clearTimeout(timer);
  }
}

function readTextCapped(res, ms) {
  return Promise.race([
    res.text(),
    sleep(Math.max(1, ms)).then(() => {
      throw new Error("body timeout");
    }),
  ]);
}

function pickFreePort() {
  return new Promise((resolve, reject) => {
    const probe = net.createServer();
    probe.on("error", reject);
    probe.listen(0, "127.0.0.1", () => {
      const addr = probe.address();
      const found = addr !== null && typeof addr === "object" ? addr.port : 0;
      probe.close((err) => {
        if (err) {
          reject(err);
        } else {
          resolve(found);
        }
      });
    });
  });
}

function exited(child, state) {
  return child.exitCode !== null || child.signalCode !== null || state.spawnError;
}

function waitForExit(child) {
  return new Promise((resolve) => {
    if (child.exitCode !== null || child.signalCode !== null) {
      resolve();
      return;
    }
    child.once("exit", () => {
      resolve();
    });
  });
}

async function stopApp(child) {
  try {
    if (child.exitCode !== null || child.signalCode !== null) {
      return;
    }
    try {
      child.kill("SIGTERM");
    } catch (err) {
    }
    await Promise.race([waitForExit(child), sleep(1000)]);
    if (child.exitCode === null && child.signalCode === null) {
      try {
        child.kill("SIGKILL");
      } catch (err) {
      }
      await Promise.race([waitForExit(child), sleep(2000)]);
    }
  } catch (err) {
  }
}

function sessionCookieValue(res) {
  let raw = [];
  if (typeof res.headers.getSetCookie === "function") {
    try {
      raw = res.headers.getSetCookie();
    } catch (err) {
      raw = [];
    }
  }
  if (!Array.isArray(raw) || raw.length === 0) {
    const single = res.headers.get("set-cookie");
    raw = single ? [single] : [];
  }
  for (const header of raw) {
    const first = String(header).split(";")[0].trim();
    if (first.indexOf("demo_session=") === 0 && first.length > "demo_session=".length) {
      return first;
    }
  }
  return null;
}

async function waitForAppAnswer(child, state, appPort, deadline) {
  const end = Date.now() + 3000;
  for (;;) {
    if (exited(child, state)) {
      return { started: false, detail: "the app did not start" };
    }
    const limit = Math.min(end, deadline);
    if (Date.now() >= limit) {
      if (exited(child, state)) {
        return { started: false, detail: "the app did not start" };
      }
      return { started: false, detail: "the app did not answer in time" };
    }
    const budget = Math.min(1500, limit - Date.now());
    try {
      await fetchWithTimeout("http://127.0.0.1:" + appPort + "/", { method: "GET" }, budget);
      return { started: true, detail: "" };
    } catch (err) {
      if (exited(child, state)) {
        return { started: false, detail: "the app did not start" };
      }
    }
    await sleep(50);
  }
}

function failure(detail) {
  return { ok: false, detail };
}

async function runCheck() {
  const deadline = Date.now() + 5000;
  const remaining = () => deadline - Date.now();
  let child = null;
  const state = { spawnError: false };
  try {
    let appPort;
    try {
      appPort = await pickFreePort();
    } catch (err) {
      return failure("the app did not start");
    }
    if (remaining() <= 0) {
      return failure("the check timed out");
    }
    const serverFile = path.join(path.resolve(appDir), "server.js");
    try {
      child = spawn(process.execPath, [serverFile, "--port", String(appPort)], {
        cwd: appDir,
        stdio: "ignore",
      });
    } catch (err) {
      return failure("the app did not start");
    }
    child.on("error", () => {
      state.spawnError = true;
    });
    liveApps.add(child);

    const ready = await waitForAppAnswer(child, state, appPort, deadline);
    if (!ready.started) {
      return failure(ready.detail);
    }
    if (remaining() <= 0) {
      return failure("the check timed out");
    }

    let loginRes;
    try {
      loginRes = await fetchWithTimeout(
        "http://127.0.0.1:" + appPort + "/api/login",
        {
          method: "POST",
          headers: { "Content-Type": "application/json" },
          body: JSON.stringify({ email: DEMO_EMAIL, password: DEMO_PASSWORD }),
        },
        Math.min(1500, remaining())
      );
    } catch (err) {
      return failure("the login request failed");
    }
    if (loginRes.status !== 200) {
      return failure("login answered " + loginRes.status);
    }
    const cookie = sessionCookieValue(loginRes);
    if (cookie === null) {
      return failure("login did not set a session cookie");
    }
    if (remaining() <= 0) {
      return failure("the check timed out");
    }

    let meRes;
    try {
      meRes = await fetchWithTimeout(
        "http://127.0.0.1:" + appPort + "/api/me",
        { method: "GET", headers: { Cookie: cookie } },
        Math.min(1500, remaining())
      );
    } catch (err) {
      return failure("the account request failed");
    }
    if (meRes.status !== 200) {
      return failure("account answered " + meRes.status);
    }
    let parsed;
    try {
      const text = await readTextCapped(meRes, Math.min(1500, remaining()));
      parsed = JSON.parse(text);
    } catch (err) {
      return failure("the account answer was not JSON");
    }
    if (parsed === null || typeof parsed !== "object" || typeof parsed.name !== "string" || parsed.name.length === 0) {
      return failure("the account answer had no name");
    }
    return { ok: true, name: parsed.name, detail: "" };
  } catch (err) {
    return failure("unexpected error");
  } finally {
    if (child !== null) {
      await stopApp(child);
      liveApps.delete(child);
    }
  }
}

async function readReadme() {
  try {
    const text = await fs.readFile(path.join(appDir, "README.md"), "utf8");
    return String(text).slice(0, 20000);
  } catch (err) {
    return null;
  }
}

function renderPage(result, readme) {
  let headline;
  if (result.ok) {
    headline = '<h1 class="ok">Welcome, ' + escapeHtml(result.name) + "</h1>";
  } else {
    headline =
      '<h1 class="fail">Something went wrong</h1>\n' +
      '<p class="detail">' +
      escapeHtml(result.detail) +
      "</p>";
  }
  const shown = readme === null ? "(no README.md in this folder)" : readme;
  return (
    "<!doctype html>\n" +
    '<html lang="en">\n' +
    "<head>\n" +
    '<meta charset="utf-8">\n' +
    '<meta http-equiv="refresh" content="2">\n' +
    "<title>Login pane</title>\n" +
    "<style>\n" +
    "body { font-family: -apple-system, system-ui, sans-serif; margin: 48px; }\n" +
    "h1 { font-size: 48px; }\n" +
    "h1.ok { color: green; }\n" +
    "h1.fail { color: red; }\n" +
    ".detail { color: grey; font-size: 20px; }\n" +
    "pre { font-size: 24px; white-space: pre-wrap; overflow-wrap: break-word; }\n" +
    "</style>\n" +
    "</head>\n" +
    "<body>\n" +
    headline +
    "\n<pre>" +
    escapeHtml(shown) +
    "</pre>\n" +
    "</body>\n" +
    "</html>\n"
  );
}

const server = http.createServer((req, res) => {
  const rawUrl = typeof req.url === "string" ? req.url : "/";
  const cut = rawUrl.indexOf("?");
  const pathname = cut === -1 ? rawUrl : rawUrl.slice(0, cut);
  if (req.method !== "GET" || pathname !== "/") {
    res.writeHead(404, { "Content-Type": "text/plain; charset=utf-8" });
    res.end("Not Found");
    return;
  }
  (async () => {
    let result;
    try {
      result = await runCheck();
    } catch (err) {
      result = failure("unexpected error");
    }
    const readme = await readReadme();
    const page = renderPage(result, readme);
    res.writeHead(200, {
      "Content-Type": "text/html; charset=utf-8",
      "Cache-Control": "no-store",
    });
    res.end(page);
  })().catch(() => {
    try {
      res.writeHead(500, { "Content-Type": "text/plain; charset=utf-8" });
      res.end("error");
    } catch (err) {
    }
  });
});

server.listen(panePort, "127.0.0.1", () => {
  process.stdout.write("Pane listening on http://127.0.0.1:" + panePort + "/\n");
});
