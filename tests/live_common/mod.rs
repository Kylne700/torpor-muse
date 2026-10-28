//! What the live tests share: the key, the state dir, the spend file and the stop before 5 dollars.
#![allow(dead_code)]

use super::writer_common::{Sandbox, real_engine};
use serde_json::Value;
use std::fs;
use std::io::Write;
use std::path::PathBuf;

const STOP_AT_USD: f64 = 4.8;

pub struct Live {
    key: String,
    pub state: PathBuf,
    spend: PathBuf,
}

/// What one run came to.
pub struct Case {
    pub label: String,
    pub out: String,
    pub code: i32,
    pub secs: f64,
    pub usd: f64,
    pub calls: usize,
}

impl Live {
    pub fn new() -> Live {
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
                "# S3b-writer spend ledger (every response, failed or retried too)\n\n| time (UTC) | run | model | status | input | cached | output | usd | estimate | note |\n|---|---|---|---|---|---|---|---|---|---|\n",
            )
            .unwrap();
        }
        live
    }

    pub fn total(&self) -> f64 {
        fs::read_to_string(self.state.join("ledger.jsonl"))
            .unwrap_or_default()
            .lines()
            .filter_map(|l| serde_json::from_str::<Value>(l).ok()?["usd"].as_f64())
            .sum()
    }

    /// A fresh synthetic workspace with the real engine and a history ending in `praise`.
    pub fn sandbox(&self, praise: &str) -> Sandbox {
        let mut sb = Sandbox::new("live");
        sb.engine = Some(real_engine().expect("the live runs need the engine (TORPOR_BIN)"));
        sb.state_override = Some(self.state.clone());
        sb.engine_history(praise);
        sb
    }

    /// The run the Muse hook would start after `praise`, against the real API.
    pub fn ask(&self, sb: &Sandbox, label: &str, praise: &str) -> Case {
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
                "| {} | {label} | {} | {} | {} | {} | {} | {:.6} | {} | {} |",
                row["at"].as_str().unwrap_or(""),
                row["model"].as_str().unwrap_or(""),
                row["status"].as_str().unwrap_or(""),
                g("input"),
                g("cached"),
                g("output"),
                row["usd"].as_f64().unwrap_or(0.0),
                row["est"],
                row["note"].as_str().unwrap_or("")
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
