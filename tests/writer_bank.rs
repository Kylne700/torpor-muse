//! The paid task bank (eval/): the writer on the bank's apps, then its check on builds that break or keep the behaviour.

mod live_common;
mod writer_common;

use live_common::*;
use serde_json::{Value, json};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use writer_common::*;

fn eval_dir() -> PathBuf {
    repo_root().join("eval")
}

/// Runs a program with a clean environment; returns (exit code, stdout, stderr).
fn run(prog: &str, args: &[&str], dir: &Path) -> (i32, String, String) {
    let o = Command::new(prog)
        .args(args)
        .current_dir(dir)
        .env_clear()
        .env("PATH", std::env::var("PATH").unwrap_or_default())
        .env("TMPDIR", std::env::temp_dir())
        .output()
        .unwrap_or_else(|e| panic!("run {prog}: {e}"));
    (
        o.status.code().unwrap_or(-1),
        String::from_utf8_lossy(&o.stdout).into_owned(),
        String::from_utf8_lossy(&o.stderr).into_owned(),
    )
}

struct Task {
    id: String,
    app: String,
    first_prompt: String,
    praise: String,
    behaviours: Vec<String>,
}

fn load_task(id: &str) -> Task {
    let text = fs::read_to_string(eval_dir().join("tasks").join(id).join("task.json")).unwrap();
    let v: Value = serde_json::from_str(&text).unwrap();
    let s = |x: &Value| x.as_str().unwrap().to_string();
    Task {
        id: id.into(),
        app: s(&v["app"]),
        first_prompt: s(&v["turns"][0]["prompt"]),
        praise: s(&v["praise"]["words"]),
        behaviours: v["praise"]["behaviours"]
            .as_array()
            .map(|a| a.iter().map(&s).collect())
            .unwrap_or_default(),
    }
}

/// The task's start state as the sandbox workspace (a git repo with one commit and the `.synthetic` marker).
fn eval_workspace(task: &Task) -> Sandbox {
    let sb = Sandbox::new(&task.id);
    fs::remove_dir_all(&sb.ws).unwrap();
    let (code, out, err) = run(
        "node",
        &["make-workspace.mjs", &task.id, sb.ws.to_str().unwrap()],
        &eval_dir(),
    );
    assert_eq!(code, 0, "make-workspace: {out}{err}");
    sb
}

fn patch(sb: &Sandbox, task: &str, name: &str) {
    let p = eval_dir().join("tasks").join(task).join(name);
    let args = ["apply", "--recount", "-C1", "--whitespace=nowarn"];
    sb.git(&[args.as_slice(), &[p.to_str().unwrap()]].concat());
}

/// Back to the start state.
fn reset(sb: &Sandbox) {
    sb.git(&["checkout", "-q", "--", "."]);
    sb.git(&["clean", "-fdq"]);
}

/// The workspace plus a real-engine history: turn 1 starts on the start state and ends on the reference build (the
/// stand-in for "the AI did the work"), then the praise starts turn 2.
fn eval_sandbox(live: &Live, task: &Task) -> Sandbox {
    let mut sb = eval_workspace(task);
    sb.engine = Some(real_engine().expect("the live runs need the engine (TORPOR_BIN)"));
    sb.state_override = Some(live.state.clone());
    sb.hook("SessionStart", json!({"source": "startup"}));
    sb.hook("UserPromptSubmit", json!({"prompt": task.first_prompt}));
    patch(&sb, &task.id, "reference.patch");
    sb.hook("Stop", json!({"stop_hook_active": false}));
    sb.hook("UserPromptSubmit", json!({"prompt": task.praise}));
    sb
}

/// The held-out oracle's failed checks on the workspace as it is (the ground truth, no model involved).
fn oracle_failed(task: &Task, sb: &Sandbox) -> Vec<String> {
    let script = format!("oracles/{}.mjs", task.app);
    let args = [
        script.as_str(),
        "--task",
        &task.id,
        "--dir",
        sb.ws.to_str().unwrap(),
    ];
    let (code, out, err) = run("node", &args, &eval_dir());
    let v: Value = serde_json::from_str(out.lines().last().unwrap_or(""))
        .unwrap_or_else(|_| panic!("oracle exit {code}: {out}{err}"));
    let ids = v["failed"].as_array().unwrap();
    ids.iter()
        .map(|x| x.as_str().unwrap_or("?").to_string())
        .collect()
}

/// `torpor check 1` on the workspace as it is: (PASS, FAIL, FLAKY or UNRUNNABLE; the engine's line).
fn check(sb: &Sandbox) -> (String, String) {
    let (_, out, _) = sb.torpor(&["check", "1"], None);
    let line = out
        .lines()
        .rev()
        .find(|l| l.starts_with("Promise #"))
        .unwrap_or("")
        .to_string();
    let word = ["PASS", "FAIL", "FLAKY", "UNRUNNABLE"]
        .into_iter()
        .find(|w| line.contains(&format!(": {w}")))
        .unwrap_or("?");
    (word.to_string(), line)
}

/// One string edit; `from` must occur exactly once in the file.
struct Edit {
    file: &'static str,
    from: &'static str,
    to: &'static str,
}

/// A change to the control task's reference build: it breaks the praised behaviour, or it does not matter.
struct Mutation {
    name: &'static str,
    breaks: bool,
    edits: &'static [Edit],
}

const fn ed(file: &'static str, from: &'static str, to: &'static str) -> Edit {
    Edit { file, from, to }
}

const fn brk(name: &'static str, edits: &'static [Edit]) -> Mutation {
    Mutation {
        name,
        breaks: true,
        edits,
    }
}

const fn fine(name: &'static str, edits: &'static [Edit]) -> Mutation {
    Mutation {
        name,
        breaks: false,
        edits,
    }
}

/// Base: notes-ctrl (login in any capitalisation).
const NOTES: &[Mutation] = &[
    brk(
        "capital letters refused",
        &[ed(
            "server.js",
            "store.findUser(String(username).toLowerCase())",
            "store.findUser(username)",
        )],
    ),
    brk(
        "right password refused",
        &[ed(
            "server.js",
            "if (!user || !auth.verifyPassword(",
            "if (!user || auth.verifyPassword(",
        )],
    ),
    brk(
        "login sets no cookie",
        &[ed(
            "server.js",
            "{ 'Set-Cookie': `sid=${sid}; HttpOnly; Path=/` }",
            "{}",
        )],
    ),
    brk(
        "login answers 201",
        &[ed(
            "server.js",
            "return send(res, 200, { username: user.username }, {",
            "return send(res, 201, { username: user.username }, {",
        )],
    ),
    brk(
        "names upper-cased",
        &[ed(
            "server.js",
            "String(username).toLowerCase()",
            "String(username).toUpperCase()",
        )],
    ),
    brk(
        "session never recognised",
        &[ed(
            "lib/auth.js",
            "const userId = match && sessions.get(match[1]);",
            "const userId = match && null;",
        )],
    ),
    fine(
        "home page wording",
        &[ed(
            "server.js",
            "Log in with POST /login, then use /notes.",
            "Sign in with POST /login, then open /notes.",
        )],
    ),
    fine(
        "comment added",
        &[ed(
            "server.js",
            "const PORT = Number(process.env.PORT) || 3000;",
            "const PORT = Number(process.env.PORT) || 3000; // the port to listen on",
        )],
    ),
    fine(
        "404 wording",
        &[ed(
            "server.js",
            "  return send(res, 404, { error: 'not found' });\n}\n\nhttp",
            "  return send(res, 404, { error: 'no such route' });\n}\n\nhttp",
        )],
    ),
    fine(
        "register error wording",
        &[ed(
            "server.js",
            "'invalid username or password'",
            "'username or password not valid'",
        )],
    ),
    fine(
        "new /ping route",
        &[ed(
            "server.js",
            "path === '/health') return send(res, 200, { status: 'ok' });",
            "path === '/health') return send(res, 200, { status: 'ok' });\n  if (method === 'GET' && path === '/ping') return send(res, 200, { pong: true });",
        )],
    ),
    fine(
        "log line wording",
        &[ed(
            "server.js",
            "notes listening on",
            "notes service ready on",
        )],
    ),
];

/// Base: shop-ctrl (adding the same product twice gives one line).
const SHOP: &[Mutation] = &[
    brk(
        "second add makes a new line",
        &[ed(
            "lib/carts.js",
            "  if (line) {\n    line.qty += qty;",
            "  if (false) {\n    line.qty += qty;",
        )],
    ),
    brk(
        "second add overwrites the quantity",
        &[ed("lib/carts.js", "line.qty += qty;", "line.qty = qty;")],
    ),
    brk(
        "line total ignores the quantity",
        &[ed(
            "server.js",
            "lineCents: product.priceCents * line.qty,",
            "lineCents: product.priceCents,",
        )],
    ),
    brk(
        "cart total adds unit prices",
        &[ed(
            "server.js",
            "sum + item.lineCents",
            "sum + item.priceCents",
        )],
    ),
    brk(
        "different products share one line",
        &[ed(
            "lib/carts.js",
            "  const line = findLine(customer, productId);\n  if (line) {",
            "  const line = linesOf(customer)[0];\n  if (line) {",
        )],
    ),
    brk(
        "cart shows quantity 1",
        &[ed("server.js", "      qty: line.qty,\n", "      qty: 1,\n")],
    ),
    fine(
        "home page wording",
        &[ed(
            "server.js",
            "Browse /products, then fill your cart with POST /cart/items.",
            "Look at /products, then put things in your cart with POST /cart/items.",
        )],
    ),
    fine(
        "comment added",
        &[ed(
            "server.js",
            "// shop: a small shop API.",
            "// shop: a small shop API (no database needed).",
        )],
    ),
    fine(
        "404 wording",
        &[ed(
            "server.js",
            "  send(res, 404, { error: 'not found' });\n}\n\nhttp",
            "  send(res, 404, { error: 'no such route' });\n}\n\nhttp",
        )],
    ),
    fine(
        "empty cart wording",
        &[ed(
            "server.js",
            "'the cart is empty'",
            "'your cart is empty'",
        )],
    ),
    fine(
        "new /ping route",
        &[ed(
            "server.js",
            "pathname === '/health') return health(ctx);",
            "pathname === '/health') return health(ctx);\n  if (method === 'GET' && pathname === '/ping') return send(res, 200, { pong: true });",
        )],
    ),
    fine(
        "log line wording",
        &[ed("server.js", "shop listening on", "shop is up on")],
    ),
];

/// Base: todo-ctrl (marking a todo done).
const TODO: &[Mutation] = &[
    brk(
        "done is ignored",
        &[ed(
            "server.js",
            "  if (typeof body.done === 'boolean') todo.done = body.done;\n",
            "",
        )],
    ),
    brk(
        "done is inverted",
        &[ed(
            "server.js",
            "todo.done = body.done;",
            "todo.done = !body.done;",
        )],
    ),
    brk(
        "patch answers 201",
        &[ed(
            "server.js",
            "  db.save();\n  send(res, 200, present(todo));",
            "  db.save();\n  send(res, 201, present(todo));",
        )],
    ),
    brk(
        "done is never shown",
        &[ed(
            "server.js",
            "    done: todo.done,\n  };",
            "    done: false,\n  };",
        )],
    ),
    brk(
        "done is shown as text",
        &[ed(
            "server.js",
            "    done: todo.done,\n  };",
            "    done: String(todo.done),\n  };",
        )],
    ),
    brk(
        "done cannot be unset",
        &[ed(
            "server.js",
            "if (typeof body.done === 'boolean') todo.done = body.done;",
            "if (body.done === true) todo.done = true;",
        )],
    ),
    brk(
        "token never matches",
        &[ed(
            "lib/auth.js",
            "t.token === token)",
            "t.token === token + 'x')",
        )],
    ),
    fine(
        "home page wording",
        &[ed(
            "server.js",
            "Get a token with POST /token, then call /todos",
            "Ask POST /token for a token, then call /todos",
        )],
    ),
    fine(
        "comment edited",
        &[ed(
            "server.js",
            "// todo: a small todo API with bearer tokens.",
            "// todo: a tiny todo API with bearer tokens.",
        )],
    ),
    fine(
        "login-required wording",
        &[ed(
            "server.js",
            "{ error: 'login required' }",
            "{ error: 'please log in' }",
        )],
    ),
    fine(
        "title error wording",
        &[ed(
            "server.js",
            "{ error: 'title is required' }",
            "{ error: 'a title is needed' }",
        )],
    ),
    fine(
        "health answer gets a field",
        &[ed(
            "server.js",
            "send(res, 200, { status: 'ok' });",
            "send(res, 200, { status: 'ok', service: 'todo' });",
        )],
    ),
    fine(
        "log line wording",
        &[ed("server.js", "todo listening on", "todo is up on")],
    ),
];

/// The control task's reference build with the mutation's edits (each edit must match exactly once).
fn mutate(sb: &Sandbox, base: &str, m: &Mutation) {
    reset(sb);
    patch(sb, base, "reference.patch");
    for e in m.edits {
        let file = sb.ws.join(e.file);
        let text = fs::read_to_string(&file).unwrap();
        assert_eq!(
            text.matches(e.from).count(),
            1,
            "{}: {:?} in {}",
            m.name,
            e.from,
            e.file
        );
        fs::write(&file, text.replacen(e.from, e.to, 1)).unwrap();
        if e.file.ends_with(".js") {
            let (code, _, err) = run("node", &["--check", e.file], &sb.ws);
            assert_eq!(code, 0, "{}: {err}", m.name);
        }
    }
}

/// How the writer's checks did on the bank, as (got, of) pairs.
#[derive(Default)]
struct Score {
    proven: (usize, usize),
    caught: (usize, usize),
    spared: (usize, usize),
    broke: (usize, usize),
    calm: (usize, usize),
}

impl Score {
    fn print(&self, app: &str) {
        let p = |t: (usize, usize)| format!("{}/{}", t.0, t.1);
        println!(
            "LIVE SUMMARY {app}: proven {}; regression builds caught {}; unrelated-change builds not flagged {}; own breaks caught {}; own harmless edits not flagged {}",
            p(self.proven),
            p(self.caught),
            p(self.spared),
            p(self.broke),
            p(self.calm)
        );
    }
}

/// One bank task: the writer on its praise, then its check on the task's regression build (a break, or for a control
/// a harmless variant); a control also takes the string-edit mutations.
fn bank_task(live: &Live, id: &str, score: &mut Score, muts: &[Mutation]) {
    let task = load_task(id);
    let control = id.ends_with("-ctrl");
    let sb = eval_sandbox(live, &task);
    let c = live.ask(&sb, &format!("bank {id}"), &task.praise);
    let (_, shown, _) = sb.torpor(&["promise", "show", "1"], None);
    let shown: String = shown.split_whitespace().collect::<Vec<_>>().join(" ");
    println!(
        "LIVE   {id} saved: {}",
        shown.chars().take(1500).collect::<String>()
    );
    score.proven.1 += 1;
    score.proven.0 += usize::from(c.out.starts_with("proven: "));
    if c.code != 0 {
        println!("LIVE   {id}: nothing saved, so no check to run on other builds");
        return;
    }
    reset(&sb);
    patch(&sb, id, "regression.patch");
    let (word, line) = check(&sb);
    let failed = oracle_failed(&task, &sb);
    let tag = if control { "variant" } else { "regression" };
    let short: String = line.chars().take(230).collect();
    println!("LIVE   {id} {tag}: check {word}; oracle failed {failed:?} | {short}");
    let (hit, pair) = if control {
        let praised_still_holds = !failed.iter().any(|f| task.behaviours.contains(f));
        (word == "PASS" && praised_still_holds, &mut score.spared)
    } else {
        (word == "FAIL", &mut score.caught)
    };
    pair.1 += 1;
    pair.0 += usize::from(hit);
    for m in muts {
        mutate(&sb, id, m);
        let (word, line) = check(&sb);
        let failed = oracle_failed(&task, &sb);
        let pair = if m.breaks {
            &mut score.broke
        } else {
            &mut score.calm
        };
        pair.1 += 1;
        pair.0 += usize::from(word == if m.breaks { "FAIL" } else { "PASS" });
        let short: String = line.chars().take(200).collect();
        println!(
            "LIVE   {id} {} {:<34} check {word:<10} oracle failed {}{}",
            if m.breaks { "break" } else { "edit " },
            m.name,
            failed.len(),
            if word == "PASS" || word == "FAIL" {
                String::new()
            } else {
                format!(" | {short}")
            }
        );
    }
}

fn bank(app: &str, tasks: &[&str], muts: &[Mutation]) {
    let live = Live::new();
    let mut score = Score::default();
    let only = std::env::var("BANK_ONLY").unwrap_or_default();
    let wanted = |id: &str| only.is_empty() || only.split(',').any(|o| o == id);
    for id in tasks.iter().filter(|id| wanted(id)) {
        bank_task(&live, id, &mut score, &[]);
    }
    let control = format!("{app}-ctrl");
    if wanted(&control) {
        bank_task(&live, &control, &mut score, muts);
    }
    score.print(app);
    println!("LIVE total so far ${:.4}", live.total());
}

#[test]
#[ignore = "costs money: real calls"]
fn live_bank_notes() {
    bank(
        "notes",
        &[
            "notes-cookie",
            "notes-hash",
            "notes-logout",
            "notes-persist",
        ],
        NOTES,
    );
}

#[test]
#[ignore = "costs money: real calls"]
fn live_bank_shop() {
    bank(
        "shop",
        &[
            "shop-clear",
            "shop-coupon",
            "shop-customer",
            "shop-merge",
            "shop-vat",
        ],
        SHOP,
    );
}

#[test]
#[ignore = "costs money: real calls"]
fn live_bank_todo() {
    bank(
        "todo",
        &[
            "todo-logout",
            "todo-persist",
            "todo-router",
            "todo-validate",
        ],
        TODO,
    );
}

/// No model, no key: the mutations match their anchors, the edited apps still parse, and the oracle agrees which of
/// them break something.
#[test]
#[ignore = "slow: runs the oracle many times, calls no model"]
fn live_dry_mutations() {
    for (id, muts) in [
        ("notes-ctrl", NOTES),
        ("shop-ctrl", SHOP),
        ("todo-ctrl", TODO),
    ] {
        let task = load_task(id);
        let sb = eval_workspace(&task);
        patch(&sb, id, "reference.patch");
        println!(
            "DRY {id:<11} the reference build: oracle failed {:?}",
            oracle_failed(&task, &sb)
        );
        for m in muts {
            mutate(&sb, id, m);
            let failed = oracle_failed(&task, &sb);
            println!(
                "DRY {id:<11} {:<5} {:<34} oracle failed {failed:?}",
                if m.breaks { "break" } else { "edit" },
                m.name
            );
        }
    }
}
