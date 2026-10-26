const DEMO_NAME = "Sam";
const DEMO_EMAIL = "Sam.Demo@Torpor";
const DEMO_PASSWORD = "demo-password-42";

const http = require("http");

// HTTP handlers call checkLogin; nothing else here compares an email or a password.
function checkLogin(email, password) {
  if (typeof email !== "string" || typeof password !== "string") {
    return null;
  }
  if (email.trim().toLowerCase() !== DEMO_EMAIL.toLowerCase()) {
    return null;
  }
  if (password !== DEMO_PASSWORD) {
    return null;
  }
  return { name: DEMO_NAME, email: DEMO_EMAIL };
}

const sessions = new Map();
let nextSessionId = 1;

const PAGE = "<!doctype html>\n" +
'<html lang="en">\n' +
"<head>\n" +
'<meta charset="utf-8">\n' +
"<title>Login demo</title>\n" +
"<style>\n" +
"body { font-family: sans-serif; font-size: 30px; margin: 40px; }\n" +
"input, button { font-size: 30px; padding: 8px; margin: 6px 0; }\n" +
"form { max-width: 480px; }\n" +
"</style>\n" +
"</head>\n" +
"<body>\n" +
"<h1>Login demo</h1>\n" +
'<form id="login-form">\n' +
'<label for="email">Email</label><br>\n' +
'<input type="text" id="email" name="email" autocomplete="username"><br>\n' +
'<label for="password">Password</label><br>\n' +
'<input type="password" id="password" name="password" autocomplete="current-password"><br>\n' +
'<button type="submit">Sign in</button>\n' +
"</form>\n" +
'<p id="message"></p>\n' +
"<script>\n" +
'document.getElementById("login-form").addEventListener("submit", async (e) => {\n' +
"  e.preventDefault();\n" +
'  const message = document.getElementById("message");\n' +
"  try {\n" +
'    const email = document.getElementById("email").value;\n' +
'    const password = document.getElementById("password").value;\n' +
'    const loginRes = await fetch("/api/login", { method: "POST", headers: { "Content-Type": "application/json" }, body: JSON.stringify({ email, password }) });\n' +
'    if (loginRes.status !== 200) { message.textContent = "Something went wrong"; return; }\n' +
'    const meRes = await fetch("/api/me");\n' +
'    if (meRes.status !== 200) { message.textContent = "Something went wrong"; return; }\n' +
"    const me = await meRes.json();\n" +
'    if (!me || typeof me.name !== "string") { message.textContent = "Something went wrong"; return; }\n' +
'    document.getElementById("login-form").remove();\n' +
'    message.textContent = "Welcome, " + me.name;\n' +
"  } catch (err) {\n" +
'    message.textContent = "Something went wrong";\n' +
"  }\n" +
"});\n" +
"</script>\n" +
"</body>\n" +
"</html>\n";

function sendJson(res, status, obj, extraHeaders) {
  const body = JSON.stringify(obj);
  const headers = { "Content-Type": "application/json; charset=utf-8" };
  if (extraHeaders) {
    for (const key of Object.keys(extraHeaders)) {
      headers[key] = extraHeaders[key];
    }
  }
  res.writeHead(status, headers);
  res.end(body);
}

function getSessionToken(req) {
  const header = req.headers.cookie;
  if (typeof header !== "string") {
    return null;
  }
  const parts = header.split(";");
  for (let i = 0; i < parts.length; i++) {
    const pair = parts[i].split("=");
    if (pair.length < 2) {
      continue;
    }
    const name = pair[0].trim();
    if (name !== "demo_session") {
      continue;
    }
    const value = pair.slice(1).join("=").trim();
    return value;
  }
  return null;
}

function parsePort() {
  const args = process.argv.slice(2);
  let raw;
  let flagPresent = false;
  for (let i = 0; i < args.length; i++) {
    const a = args[i];
    if (a === "--port") {
      flagPresent = true;
      raw = args[i + 1];
      break;
    }
    if (typeof a === "string" && a.slice(0, 7) === "--port=") {
      flagPresent = true;
      raw = a.slice(7);
      break;
    }
  }
  if (!flagPresent) {
    if (process.env.PORT !== undefined && process.env.PORT !== "") {
      raw = process.env.PORT;
    } else {
      raw = "3000";
    }
  }
  const s = String(raw).trim();
  let digits = s.length > 0;
  for (let i = 0; i < s.length; i++) {
    const c = s.charCodeAt(i);
    if (c < 48 || c > 57) {
      digits = false;
      break;
    }
  }
  const n = digits ? Number(s) : NaN;
  if (!digits || !Number.isInteger(n) || n < 1 || n > 65535) {
    process.stderr.write("Invalid port: expected a whole number from 1 to 65535\n");
    process.exit(2);
  }
  return n;
}

const port = parsePort();

const server = http.createServer((req, res) => {
  const rawUrl = typeof req.url === "string" ? req.url : "/";
  const q = rawUrl.indexOf("?");
  const path = q === -1 ? rawUrl : rawUrl.slice(0, q);

  if (req.method === "GET" && path === "/") {
    res.writeHead(200, { "Content-Type": "text/html; charset=utf-8" });
    res.end(PAGE);
    return;
  }

  if (req.method === "POST" && path === "/api/login") {
    let size = 0;
    const chunks = [];
    let answered = false;
    let tooBig = false;
    req.on("data", (chunk) => {
      if (answered) {
        return;
      }
      size += chunk.length;
      if (size > 4096) {
        tooBig = true;
        return;
      }
      chunks.push(chunk);
    });
    req.on("end", () => {
      if (answered) {
        return;
      }
      if (tooBig) {
        sendJson(res, 400, { ok: false });
        return;
      }
      let parsed;
      try {
        const text = Buffer.concat(chunks).toString("utf8");
        parsed = JSON.parse(text);
      } catch (err) {
        sendJson(res, 400, { ok: false });
        return;
      }
      if (parsed === null || typeof parsed !== "object" || Array.isArray(parsed)) {
        sendJson(res, 400, { ok: false });
        return;
      }
      const account = checkLogin(parsed.email, parsed.password);
      if (account === null) {
        sendJson(res, 401, { ok: false });
        return;
      }
      const token = "demo-" + (nextSessionId++);
      sessions.set(token, true);
      sendJson(res, 200, { ok: true }, {
        "Set-Cookie": "demo_session=" + token + "; HttpOnly; SameSite=Lax; Path=/",
      });
    });
    req.on("error", () => {
      if (!answered) {
        answered = true;
        try {
          sendJson(res, 400, { ok: false });
        } catch (err) {
        }
      }
    });
    return;
  }

  if (req.method === "GET" && path === "/api/me") {
    const token = getSessionToken(req);
    if (token !== null && sessions.has(token)) {
      sendJson(res, 200, { name: DEMO_NAME, email: DEMO_EMAIL });
    } else {
      sendJson(res, 401, { ok: false });
    }
    return;
  }

  res.writeHead(404, { "Content-Type": "text/plain; charset=utf-8" });
  res.end("Not Found");
});

process.on("SIGTERM", () => {
  process.exit(0);
});
process.on("SIGINT", () => {
  process.exit(0);
});

server.listen(port, "127.0.0.1", () => {
  process.stdout.write("Listening on http://127.0.0.1:" + port + "/\n");
});
