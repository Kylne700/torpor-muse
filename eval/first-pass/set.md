# First-pass eval set

Forty prompts, fixed before the first keyed call and never changed after it. Written for this stage; nothing in them
comes from a real session. A `C` prompt says that something works now (alone, or with a request after it). An `N`
prompt does not: a request, a question, a complaint, a plan, pasted error text. `b` marks the ten claims and the ten
others that also get today's single call (the baseline). `\n` is a line break inside one prompt.

The line format is `<id> [b] | <prompt>`; `run.mjs` reads exactly these lines.

## Say that something works (20)

C01 b | login works now
C02 | Great, signing in works and the account page shows the right name.
C03 b | I just tried it by hand and logging in with the demo user works.
C04 | the login is fixed, it returns the user now
C05 b | Sign-in works. Next, please add a logout button to the account page.
C06 | login works now and /api/me answers 200 after signing in
C07 b | yes! the session cookie is set and the account page loads
C08 | Everything works: login, then the profile call, no errors.
C09 b | wrong passwords are rejected with a 401 now, that part works
C10 | perfect, it works - I can log in and see my name
C11 b | that fixed it, login is working again. can you now update the README?
C12 | the health check answers ok now
C13 b | logn works now
C14 | the acount page works, it shows my name after loging in
C15 | Tested it by hand and signing in works, so let's move on to the styles.
C16 b | ההתחברות עובדת עכשיו
C17 | הכניסה עם משתמש הדמו עובדת, והעמוד האישי מציג את השם שלי.
C18 b | מעולה, זה עובד! אפשר להתחבר ולראות את החשבון. עכשיו תוסיף כפתור יציאה.
C19 | Looks like login is working now; the old 500 error is gone.
C20 b | logging out works too: after it, the account page says you are not signed in

## Do not say that something works (20)

N01 b | does login work?
N02 | make login work
N03 b | login doesn't work
N04 b | continue
N05 | yes do it
N06 b | please add a logout button to the account page
N07 b | TypeError: Cannot read properties of undefined (reading 'name')\n    at handleLogin (/app/server.js:88:21)\n    at Server.<anonymous> (/app/server.js:140:5)
N08 | why does the login answer 500 when I send an empty body?
N09 b | can you check whether the session cookie is HttpOnly?
N10 | Error: listen EADDRINUSE: address already in use :::3000
N11 | next I want a page that lists all users, let's plan it first
N12 b | ok thanks, that is all for today
N13 | I wonder whether login will keep working once we add a database
N14 b | login still fails with the demo user, it says wrong password every time
N15 | the account page should show the user's name after login
N16 | run the tests and tell me which ones fail
N17 b | rename the variable tok to token in server.js
N18 | what does the /api/me route return when I am not signed in?
N19 | undo that, it broke the login
N20 b | האם ההתחברות עובדת?
