# How demo/ was built: prompts and transcripts

`app/server.js`, `pane.js` and `README.md` in this folder were written by Muse Code from the English prompts below, during the event.
No demo code was pasted into a prompt and no line was edited by a person.

| Setting | Value |
|---|---|
| Tool | Muse Code 1.4.13 (1.4.13-R6996.1), `exec` mode, a new session for every prompt |
| Model | muse-spark-1.3-contributor (Meta API) |
| Flags | `--provider meta --model muse-spark-1.3-contributor --contributor-mode off --approval-judge off --reasoning-effort medium --disable-web-tools --disable-reminders --trust-workspace --disable-approval --max-model-steps N --prompt-file FILE` |
| Workspace | a new, empty folder marked as synthetic; plain Muse Code, no Torpor hook installed |
| Environment | cleared environment, its own temporary HOME, auto-update off, the API key given only as an environment variable of the one command |

## Runs

All numbers are for muse-spark-1.3-contributor. Every run ended with the terminal state shown.

| # | What | Step cap | Model calls | Ended | Sandbox |
|---|---|---|---|---|---|
| 00 | model id check | 2 | 1 | completed | no sandbox |
| 01 | builds app/server.js | 20 | 8 | completed | no sandbox |
| 02 | builds pane.js | 30 | 20 | completed | no sandbox |
| 03 | checks that Muse's bash tool works inside bubblewrap | 4 | 2 | completed | bubblewrap |
| 04 | follow-up: fixes the pane's own shutdown | 30 | 14 | completed | bubblewrap |
| 05 | writes README.md | 12 | 3 | completed | bubblewrap |

Notes:
- Runs 00 to 02 had an isolated HOME but no file-system sandbox. Prompt 02 told Muse to make its scratch copies in a temporary directory outside the workspace; Muse did, and removed them, except for two small test scripts that were moved out of /tmp by hand and are not part of the demo. From run 03 on, every run was inside bubblewrap (everything outside one temporary folder read-only, a private /tmp), and the prompts say: file tools write only inside the workspace, scratch files go under $TMPDIR.
- Prompt 04 is a follow-up. Testing the finished pane showed that an app which ignores SIGTERM survived the pane's own shutdown.
- Two offline start-up checks with the `echo` provider (no model, no prompt to a model) are not listed.

## Transcripts

`NN-name.jsonl` in this folder, one per run, numbered as above. Each line is one event.
- Kept: the prompt, reasoning summaries, assistant text, tool calls (name and arguments), tool results (cut at 6000 characters), the token usage of each model call, and how the run ended.
- Dropped: Meta's own system and developer messages, the tool definitions, encrypted reasoning blobs, and all other bookkeeping events. Also dropped: the content of the two scratch test scripts that run 02 wrote outside the workspace (the tool call is kept, with a marker where the content was).
- Redacted: the user's home, project and work folders (`<HOME>`, `<PROJECT>`, `<MUSE-INSTALL>`, `<WORKSPACE>`, `<WORK>`), the operating-system user name (`<USER>`), session, task and record ids (`<id>`), provider response ids (`<resp>`).

## Prompts (verbatim)

Each prompt is exactly the text Muse received, except that the final line break of the prompt file is not shown.

### 00-ping (step cap 2)

~~~~text
Reply with the single word: ok. Do not use any tools.
~~~~

### 01-server (step cap 20)

~~~~text
Please build a small login demo app. This workspace is empty. Create exactly one file, demo/app/server.js, and no other file: no package.json, no tests, no README. Do not run git. Plain Node.js (18 or newer), built-in modules only, one file.

How it starts
- `node demo/app/server.js --port 4000` listens on that port, on 127.0.0.1 only. Without --port it uses the PORT environment variable, and without that it uses 3000. A port that is not a whole number from 1 to 65535 prints one line to stderr and exits with code 2.
- It must be ready in well under one second and print exactly one line to stdout saying which address it listens on.
- It exits at once (within 100 ms) on SIGTERM or SIGINT.
- It writes no files, keeps its state in memory only, and behaves the same on every run: no Math.random, no clock, no timers. A session token is simply a counter-based string, for example "demo-1" for the first sign-in, "demo-2" for the second.

The HTTP contract (exactly this)
- GET / answers 200 with the HTML login page.
- POST /api/login takes a JSON body {"email": "...", "password": "..."}. If the pair belongs to the demo user it answers 200 with the JSON {"ok":true} and sets a cookie named demo_session (HttpOnly, SameSite=Lax, Path=/). Otherwise it answers 401 with {"ok":false}. A body that is not valid JSON, is not an object, or is longer than 4096 bytes answers 400 with {"ok":false}. It never crashes on bad input.
- GET /api/me with the demo_session cookie of a signed-in visitor answers 200 with the JSON {"name":"Sam","email":"Sam.Demo@Torpor"}. With no cookie or an unknown cookie it answers 401 with {"ok":false}.
- Every other method or path answers 404.
- JSON answers use content type application/json; the page uses text/html; charset=utf-8.

The demo user (a fixed fake account, written at the top of the file as constants)
- name: Sam
- email: Sam.Demo@Torpor
- password: demo-password-42

The login logic lives in ONE small top-level function named checkLogin(email, password). It returns the demo user's account object ({name, email}) when the login is good and null otherwise. It compares the email ignoring letter case and ignoring spaces around it, and compares the password exactly. The HTTP handlers call checkLogin, and nothing else in the file compares an email or a password. Put a one-line comment above it saying that. Do not validate the email format anywhere: no regular expression for emails, nothing that insists on a dot after the @ sign, because the demo address has none. Do not write any other email-looking text anywhere in the file, not even as a placeholder in the form.

The login page (GET /)
- A title, a form with a plain text input for the email (id "email"), a password input (id "password") and a button "Sign in". The email field is a text input, not type=email. Large readable type (about 28px or more), simple inline CSS, nothing loaded from outside.
- A single element with id "message" that is empty at first.
- The small script is inline in the page. On submit it posts the two values as JSON to /api/login. If the answer is 200 it then asks /api/me and replaces the form with the text "Welcome, " followed by the name from /api/me, shown in the "message" element. On any failure at all (wrong password, unknown email, 400, 401, 404, a network error, an answer it cannot read, anything unexpected) it shows exactly "Something went wrong" in the "message" element and nothing more specific, so the page never says which part was wrong. Put the text in with textContent, never innerHTML.

When you have written the file, try it yourself: start it on a spare port in the background, call it for a good login, a wrong password, a bad JSON body, /api/me with and without the cookie, and an unknown path, then stop it and make sure no server process of yours is left running. If you cannot run it in this environment, say so plainly. Finish with a short summary of what you built and what you tested.
~~~~

### 02-pane (step cap 30)

~~~~text
Please build the second file of the login demo: demo/pane.js. The workspace already holds demo/app/server.js, the little login app. Create exactly one new file, demo/pane.js, and no other file. Do not change demo/app/server.js and do not run git. Plain Node.js (18 or newer), built-in modules only (the global fetch is available), one file.

What it is
A tiny web page for the right half of a screen recording. It shows, live, whether the login of an app folder works, and it shows that folder's README.md as text.

Usage
`node demo/pane.js DIR PORT`. DIR is a folder that holds an app file named server.js and maybe a README.md (demo/app is such a folder, though it has no README). PORT is the port the pane itself listens on, on 127.0.0.1 only. A missing argument, an extra argument, or a PORT that is not a whole number from 1 to 65535 prints one usage line to stderr and exits with code 2. On SIGTERM or SIGINT it stops any app it has started and exits at once.

Only GET / is served. Any other path or method answers 404 and starts nothing.

What one request does
For every GET / the pane makes a complete, fresh check. Nothing is cached, no state is kept between requests and there is no file watcher, so the page always shows what is on disk at that moment.
1. Pick a free port and start the app: the same node executable (process.execPath) running DIR/server.js with the arguments --port and that port, with DIR as the working directory and its output ignored.
2. Wait until the app answers GET / (poll about every 50 ms). Give up at once if the app process has already exited, or after 3 seconds.
3. Sign in as the demo user and ask for the account, using the app's HTTP contract, restated here. POST /api/login takes a JSON body {"email": "...", "password": "..."}; a good login answers 200 and sets a cookie named demo_session, a bad one answers 401 (or 400). GET /api/me, sent with the demo_session cookie (just name=value from the Set-Cookie header, nothing after the first semicolon), answers 200 with JSON containing "name" and "email", or 401 without a good cookie. The demo user is a fixed fake account, written at the top of pane.js as constants: the pane signs in with the email written in all lowercase, sam.demo@torpor, on purpose (the app is expected to ignore letter case), and the password demo-password-42.
4. The verdict. It is good only if the login answered 200 and then /api/me answered 200 with JSON whose "name" is a non-empty string. The page then shows the headline "Welcome, " followed by that name, in green. Every other outcome is a failure: the app file missing, the app crashing or not starting, no answer in time, a 401 or 404, JSON it cannot read, anything unexpected. The page then shows the headline "Something went wrong" in red and, below it, one small grey line naming which step failed (for example "login answered 401" or "the app did not start").
5. Stop the app again before the request is answered, on every path: success, failure, timeout, any exception. Send it SIGTERM, send SIGKILL if it is still alive after 1 second, and wait until it has exited. Every fetch has its own timeout of 1.5 seconds and the whole check has an overall limit of 5 seconds, so an app that hangs can neither hang the pane nor be left running.

The page
- HTML, content type text/html; charset=utf-8, header Cache-Control: no-store, and a meta refresh that reloads the page every 2 seconds.
- Large type for a screen recording: headline about 48px, README text about 24px, system font, generous margins.
- Below the headline, the content of DIR/README.md read fresh from disk on every request, shown as plain text in a pre element with long lines wrapped, at most the first 20000 characters. If the file is missing or unreadable, show the line "(no README.md in this folder)".
- Escape every dynamic string that goes into the HTML (the README text, the name, the failure line, anything that came from the app) for the five characters & < > " and '.

Read-only: the pane never writes, creates or deletes any file, in DIR or anywhere else. Do not write any other email-looking text anywhere in the file, and do not validate the email format.

When you have written the file, test it yourself. Make scratch copies of demo/app under a temporary directory outside the workspace (and remove them afterwards): one good copy, one where login is broken on purpose, one whose server.js has a syntax error, one whose server never answers requests, plus a folder with no server.js. Run the pane against each, fetch its page, and check the headline and the colour class or style. Check that the README text is shown and escaped (put a line with an HTML tag in a scratch README.md). At the end make sure that no server.js or pane.js process of yours is still running. If you cannot run something in this environment, say so plainly. Finish with a short summary of what you built and what you tested.
~~~~

### 03-confine-probe (step cap 4)

~~~~text
Use your bash tool once to run this command: echo "$TMPDIR"; ls /tmp
Then reply with its output and nothing else. Do not use any other tool and do not create any file.
~~~~

### 04-pane-kill (step cap 30)

~~~~text
One small fix in demo/pane.js, the live pane of the login demo (the workspace also holds demo/app/server.js, the little login app). Change only demo/pane.js: do not touch demo/app/server.js, create no other file in the workspace, and do not run git. Plain Node.js, built-in modules only, as before.

The problem
When the pane itself is stopped with SIGTERM or SIGINT in the middle of a check, it sends the app it started only SIGTERM and then exits at once. An app that ignores SIGTERM (for example one that installed a SIGTERM handler and is stuck in a busy loop) keeps running after the pane has gone. There is a second hole: a check removes its app from the set of running apps before it has finished stopping it, so a SIGTERM that reaches the pane while it is still waiting for that app to exit finds nothing to stop.

The fix
1. On SIGTERM or SIGINT the pane sends SIGKILL (not SIGTERM) to every app it has started and has not yet seen exit, and then exits at once.
2. An app stays in that set until it has really exited: take it out only after the code that stops it (SIGTERM, wait up to 1 second, then SIGKILL, then wait) has finished.
Keep everything else exactly as it is: the page, the checks, the timeouts and the usage rules.

How to test it, and where you may write
- The file tools may write only inside the workspace. Any scratch folder or scratch file goes under the directory named by the environment variable $TMPDIR and nowhere else: use the variable, never a path you typed yourself, and remove what you created there when you are done.
- In a scratch copy of demo/app make a server.js that installs a SIGTERM handler that does nothing and, when it gets a login request, spins forever in a busy loop. Start the pane on that folder, request the pane's page, and about 700 ms later send SIGTERM to the pane. Check that the pane exits within a second and that the scratch server.js process is gone within another second.
- Check that the pane on the real demo/app still shows the green welcome headline, and that no server.js or pane.js process of yours is left running at the end.
- If you cannot run something in this environment, say so plainly. Finish with a short summary of what you changed and what you tested.
~~~~

### 05-readme (step cap 12)

~~~~text
Please write the README of the login demo as demo/README.md. The workspace holds demo/app/server.js (the little login app) and demo/pane.js (a live page for the right half of a screen recording). Read both files first, then create exactly one new file, demo/README.md, and change nothing else: no other file, no git.

Plain Markdown, factual, no marketing, at most 45 lines. It must contain, in this order:
1. A heading and two or three sentences saying what the two files are. demo/app/server.js is a zero-dependency Node login app with one fixed fake demo user. demo/pane.js is a live page that, on every request, starts the app of a folder, signs in as the demo user, asks /api/me, stops the app again, and shows either a green "Welcome, <name>" or a red "Something went wrong" plus that folder's README.md as text.
2. A "Run" section with the exact commands: `node demo/app/server.js --port 3000` (the port can also come from the PORT environment variable, default 3000), and `node demo/pane.js demo/app 4173` followed by opening http://127.0.0.1:4173 in a browser (the page reloads itself every 2 seconds). It needs Node 18 or newer and nothing to install.
3. A "Demo user" section: a fixed fake account, name Sam, email Sam.Demo@Torpor, password demo-password-42. Say that the app ignores letter case and surrounding spaces in the email, that the pane signs in with the email in lowercase on purpose, and that the whole login check is the one function checkLogin(email, password) in demo/app/server.js.
4. A short "HTTP" list of what the app serves: GET /, POST /api/login (sets the cookie demo_session), GET /api/me.
5. As the very last line of the file, exactly this line, character for character:
Written by Muse Code (muse-spark-1.3-contributor) from English prompts during the event; prompts and redacted transcripts in demo/transcripts/

Do not write any other email-looking text, no other URL, no names of people, and no file paths other than the repository-relative ones named above. Finish with one sentence saying what you wrote.

Where you may write: the file tools may write only inside the workspace. If you want a scratch file for anything, put it under the directory named by the environment variable $TMPDIR (use the variable, never a path you typed yourself) and remove it afterwards.
~~~~
