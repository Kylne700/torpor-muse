# Login demo

demo/app/server.js is a zero-dependency Node login app with one fixed fake demo user.
demo/pane.js is a live page that, on every request, starts the app of a folder, signs in as the demo user, asks /api/me, stops the app again, and shows either a green "Welcome, <name>" or a red "Something went wrong" plus that folder's README.md as text.

## Run

`node demo/app/server.js --port 3000`

The port can also come from the PORT environment variable, default 3000.

`node demo/pane.js demo/app 4173`

Then open http://127.0.0.1:4173 in a browser.

The page reloads itself every 2 seconds.

It needs Node 18 or newer and nothing to install.

## Demo user

The fixed fake account has name Sam, email Sam.Demo@Torpor and password demo-password-42.

The app ignores letter case and surrounding spaces in the email.

demo/pane.js signs in with the email in lowercase on purpose.

The whole login check is the one function checkLogin(email, password) in demo/app/server.js.

## HTTP

- GET /
- POST /api/login (sets the cookie demo_session)
- GET /api/me

Written by Muse Code (muse-spark-1.3-contributor) from English prompts during the event; prompts and redacted transcripts in demo/transcripts/