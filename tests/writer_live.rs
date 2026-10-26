//! Live runs against the real Meta API (Muse Spark) on synthetic workspaces. Ignored by default, because they cost
//! money. Run by hand, with the key in the environment of this one command and a state dir shared by the runs:
//!   META_API_KEY=... LIVE_STATE=<dir> SPEND_FILE=<md file> cargo test --test writer_live -- --ignored --nocapture
//! Every response of every run lands in the binary's ledger (LIVE_STATE) and is copied to SPEND_FILE. The run stops
//! before 5 dollars. The key is passed to the child process by its environment only. Owner: S3b-writer.
mod writer_common;

use serde_json::Value;
use std::fs;
use std::io::Write;
use std::path::PathBuf;
use writer_common::*;

const STOP_AT_USD: f64 = 4.8;

struct Live {
    key: String,
    state: PathBuf,
    spend: PathBuf,
}

/// What one run came to.
struct Case {
    label: String,
    out: String,
    code: i32,
    secs: f64,
    usd: f64,
    calls: usize,
}

impl Live {
    fn new() -> Live {
        let get = |n: &str| std::env::var(n).unwrap_or_else(|_| panic!("set {n}"));
        let live = Live {
            key: get("META_API_KEY"),
            state: PathBuf::from(get("LIVE_STATE")),
            spend: PathBuf::from(get("SPEND_FILE")),
        };
        fs::create_dir_all(&live.state).unwrap();
        if !live.spend.exists() {
            fs::write(
                &live.spend,
                "# S3b-writer spend ledger (every response, failed or retried too)\n\n| time (UTC) | run | model | status | input | cached | output | usd | estimate |\n|---|---|---|---|---|---|---|---|---|\n",
            )
            .unwrap();
        }
        live
    }

    fn total(&self) -> f64 {
        fs::read_to_string(self.state.join("ledger.jsonl"))
            .unwrap_or_default()
            .lines()
            .filter_map(|l| serde_json::from_str::<Value>(l).ok()?["usd"].as_f64())
            .sum()
    }

    /// A fresh synthetic workspace with the real engine and a history ending in `praise`.
    fn sandbox(&self, praise: &str) -> Sandbox {
        let mut sb = Sandbox::new("live");
        sb.engine = Some(real_engine().expect("the live runs need the engine (TORPOR_BIN)"));
        sb.state_override = Some(self.state.clone());
        sb.engine_history(praise);
        sb
    }

    /// The run the Muse hook would start after `praise`, against the real API.
    fn ask(&self, sb: &Sandbox, label: &str, praise: &str) -> Case {
        let before = self.total();
        assert!(
            before < STOP_AT_USD,
            "spent ${before:.3} already: stop here"
        );
        let rows_before = sb.ledger().len();
        let r = sb.suggest(
            None,
            "-",
            &[
                ("TORPOR_MUSE_FROM_HOOK", "s1"),
                ("TORPOR_MUSE_WORDS", praise),
                ("META_API_KEY", &self.key),
                ("TORPOR_MUSE_DAY_CAP_USD", "4.5"),
            ],
        );
        let rows = sb.ledger();
        let mut md = fs::OpenOptions::new()
            .append(true)
            .open(&self.spend)
            .unwrap();
        for row in &rows[rows_before..] {
            let g = |k: &str| row[k].as_u64().unwrap_or(0);
            writeln!(
                md,
                "| {} | {label} | {} | {} | {} | {} | {} | {:.6} | {} |",
                row["at"].as_str().unwrap_or(""),
                row["model"].as_str().unwrap_or(""),
                row["status"].as_str().unwrap_or(""),
                g("input"),
                g("cached"),
                g("output"),
                row["usd"].as_f64().unwrap_or(0.0),
                row["est"]
            )
            .unwrap();
        }
        let usd = self.total() - before;
        let case = Case {
            label: label.into(),
            out: r.out.trim().chars().take(260).collect(),
            code: r.code,
            secs: r.secs,
            usd,
            calls: rows.len() - rows_before,
        };
        println!(
            "LIVE {:<28} exit={} {:>5.1}s ${:.4} calls={} | {}{}",
            case.label,
            case.code,
            case.secs,
            case.usd,
            case.calls,
            case.out,
            r.err.trim().chars().take(200).collect::<String>()
        );
        case
    }
}

fn percentile(sorted: &[f64], p: f64) -> f64 {
    let i = ((sorted.len() as f64 - 1.0) * p).round() as usize;
    sorted[i]
}

/// The first look at the real API: does it take the strict schema, the cache key and the low effort setting?
#[test]
#[ignore = "costs money: a real call"]
fn live_probe_one_english_praise() {
    let live = Live::new();
    let praise = "login works now";
    let sb = live.sandbox(praise);
    let c = live.ask(&sb, "probe english", praise);
    assert!(c.out.starts_with("proven: "), "{}", c.out);
}

const MATRIX: [(&str, &str, bool); 9] = [
    (
        "english",
        "Login works now: alice can sign in with her password and then see her account.",
        true,
    ),
    (
        "spanish composed",
        "El inicio de sesión funciona ahora: alice puede entrar con su contraseña y ver su cuenta.",
        true,
    ),
    (
        "spanish decomposed",
        "El inicio de sesio\u{301}n funciona ahora: alice puede entrar con su contrasen\u{303}a y ver su cuenta.",
        true,
    ),
    (
        "german composed",
        "Die Anmeldung funktioniert jetzt: alice kann sich mit ihrem Passwort einloggen und ihr Konto über die Seite sehen.",
        true,
    ),
    (
        "german decomposed",
        "Die Anmeldung funktioniert jetzt: alice kann sich mit ihrem Passwort einloggen und ihr Konto u\u{308}ber die Seite sehen.",
        true,
    ),
    (
        "spanish + english",
        "el login funciona ahora, alice puede entrar y ver su cuenta",
        true,
    ),
    (
        "german + english",
        "der login works jetzt, alice kann sich anmelden und ihr Konto sehen",
        true,
    ),
    ("english short", "login works", true),
    (
        "hebrew",
        "ההתחברות עובדת עכשיו, אליס יכולה להיכנס עם הסיסמה שלה ולראות את החשבון",
        false,
    ),
];

#[test]
#[ignore = "costs money: real calls"]
fn live_language_matrix() {
    let live = Live::new();
    let mut bad = Vec::new();
    for (label, words, proven) in MATRIX {
        let sb = live.sandbox(words);
        let c = live.ask(&sb, label, words);
        let ok = if proven {
            c.out.starts_with("proven: ")
        } else {
            c.out.starts_with("advisory: ") && c.out.len() > 30
        };
        if !ok {
            bad.push(format!("{label}: {}", c.out));
        }
    }
    println!("LIVE total so far ${:.4}", live.total());
    assert!(bad.is_empty(), "{bad:#?}");
}

#[test]
#[ignore = "costs money: real calls"]
fn live_timing_of_praise_to_proven() {
    let live = Live::new();
    let praises = [
        "login works now",
        "Great, the login is working: alice gets in and the account page knows who she is.",
        "yes! signing in works",
        "login works - I can log in as alice",
        "Perfect, logging in works now.",
        "the login page works, alice can sign in",
        "nice, sign-in is fixed",
        "I tried it and the login works for alice now",
        "login works now, notes are saved per user after signing in",
        "login is working",
    ];
    let (mut secs, mut usd, mut bad) = (Vec::new(), Vec::new(), Vec::new());
    for (i, words) in praises.iter().enumerate() {
        let sb = live.sandbox(words);
        let c = live.ask(&sb, &format!("timing {}", i + 1), words);
        if c.out.starts_with("proven: ") {
            secs.push(c.secs);
            usd.push(c.usd);
        } else {
            bad.push(format!("{}: {}", c.label, c.out));
        }
    }
    secs.sort_by(f64::total_cmp);
    usd.sort_by(f64::total_cmp);
    if !secs.is_empty() {
        println!(
            "LIVE praise-to-proven over {} proven runs: p50 {:.1}s p90 {:.1}s max {:.1}s; cost per check p50 ${:.4} max ${:.4}",
            secs.len(),
            percentile(&secs, 0.5),
            percentile(&secs, 0.9),
            secs[secs.len() - 1],
            percentile(&usd, 0.5),
            usd[usd.len() - 1]
        );
    }
    println!("LIVE total so far ${:.4}", live.total());
    assert!(bad.is_empty(), "{bad:#?}");
}
