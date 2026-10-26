//! Spend ledger: every Meta API response (failed and retried too), day cap, abort before a call that would pass it.
//! Owner: S3b-writer.
use std::fs::{DirBuilder, File, OpenOptions};
use std::io::{Read, Seek, SeekFrom, Write};
use std::os::unix::fs::{DirBuilderExt, OpenOptionsExt};
use std::path::Path;
use std::time::{SystemTime, UNIX_EPOCH};

pub const DEFAULT_CAP_USD: f64 = 2.0;

#[derive(Clone, Copy, Default, Debug, PartialEq)]
pub struct Usage {
    pub input: u64,
    pub cached: u64,
    pub output: u64,
}

/// What one response costs.
#[derive(Clone, Copy, Debug)]
pub enum Charge {
    /// The response told its token counts.
    Usage(Usage),
    /// The call was refused before any model work (HTTP 400, 402, 429).
    Free,
    /// We cannot know (dropped, hung, 5xx): charged as the worst case so the cap stays safe.
    Estimate(f64),
}

/// USD per million tokens as (cached input, fresh input, output), from the Meta price list read on 2026-10-26.
/// An id we do not know is charged at the Spark rate, the dearer one.
pub fn price(model: &str) -> (f64, f64, f64) {
    if model.ends_with("-contributor") {
        (0.002, 0.10, 0.20)
    } else {
        (0.15, 1.25, 4.25)
    }
}

pub fn cost(model: &str, u: Usage) -> f64 {
    let (cached, fresh, out) = price(model);
    let new_in = u.input.saturating_sub(u.cached);
    (new_in as f64 * fresh + u.cached as f64 * cached + u.output as f64 * out) / 1e6
}

/// The most one call can cost: a token is at least one byte, and the output budget is spent in full.
pub fn worst_case(model: &str, input_bytes: usize, max_output: u32) -> f64 {
    let (_, fresh, out) = price(model);
    (input_bytes as f64 * fresh + f64::from(max_output) * out) / 1e6
}

/// "2026-10-26T19:25:46Z" from Unix seconds (UTC, no leap seconds).
pub fn iso(secs: u64) -> String {
    let (days, rem) = ((secs / 86_400) as i64, secs % 86_400);
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = yoe + era * 400 + i64::from(m <= 2);
    format!(
        "{y:04}-{m:02}-{d:02}T{:02}:{:02}:{:02}Z",
        rem / 3600,
        rem % 3600 / 60,
        rem % 60
    )
}

pub fn now_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_secs())
}

/// The ledger file, held under an exclusive lock until dropped: the read of today's total, the decision and the
/// write of the row happen without another run in between, so two runs cannot both pass the cap.
pub struct Ledger {
    file: File,
    now: u64,
}

impl Ledger {
    pub fn open(dir: &Path) -> Result<Self, String> {
        Self::open_at(dir, now_secs())
    }

    pub fn open_at(dir: &Path, now: u64) -> Result<Self, String> {
        DirBuilder::new()
            .recursive(true)
            .mode(0o700)
            .create(dir)
            .map_err(|e| format!("cannot create the ledger folder: {e}"))?;
        let file = OpenOptions::new()
            .read(true)
            .append(true)
            .create(true)
            .mode(0o600)
            .open(dir.join("ledger.jsonl"))
            .map_err(|e| format!("cannot open the spend ledger: {e}"))?;
        file.lock()
            .map_err(|e| format!("cannot lock the spend ledger: {e}"))?;
        Ok(Self { file, now })
    }

    /// Dollars spent on the current UTC day. A line that does not parse (a write cut short) counts for nothing.
    pub fn spent_today(&mut self) -> Result<f64, String> {
        let mut raw = Vec::new();
        self.file
            .seek(SeekFrom::Start(0))
            .and_then(|_| self.file.read_to_end(&mut raw))
            .map_err(|e| format!("cannot read the spend ledger: {e}"))?;
        let day = iso(self.now)[..10].to_string();
        Ok(String::from_utf8_lossy(&raw)
            .lines()
            .filter_map(|l| serde_json::from_str::<serde_json::Value>(l).ok())
            .filter(|v| v["day"] == day.as_str())
            .filter_map(|v| v["usd"].as_f64())
            .sum())
    }

    /// Refuses when today's spend plus the worst case of the coming call would pass `cap`. Returns that worst case.
    pub fn preflight(
        &mut self,
        model: &str,
        input_bytes: usize,
        max_output: u32,
        cap: f64,
    ) -> Result<f64, String> {
        let worst = worst_case(model, input_bytes, max_output);
        let spent = self.spent_today()?;
        if spent + worst > cap {
            return Err(format!(
                "the daily spend cap would be passed: ${spent:.3} spent today (UTC), the next call could cost up to ${worst:.3}, cap ${cap:.2}. \
                 Nothing was sent. Raise the cap with TORPOR_MUSE_DAY_CAP_USD or day_cap_usd in the config."
            ));
        }
        Ok(worst)
    }

    /// Appends one row for one response; returns the dollars charged.
    pub fn record(
        &mut self,
        model: &str,
        status: &str,
        charge: Charge,
        note: &str,
    ) -> Result<f64, String> {
        let (u, usd, est) = match charge {
            Charge::Usage(u) => (u, cost(model, u), false),
            Charge::Free => (Usage::default(), 0.0, false),
            Charge::Estimate(x) => (Usage::default(), x, true),
        };
        let at = iso(self.now);
        let row = serde_json::json!({
            "at": at, "day": &at[..10], "model": model, "status": status,
            "input": u.input, "cached": u.cached, "output": u.output,
            "usd": (usd * 1e6).round() / 1e6, "est": est, "note": note,
        });
        writeln!(self.file, "{row}").map_err(|e| format!("cannot write the spend ledger: {e}"))?;
        Ok(usd)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn dir(name: &str) -> std::path::PathBuf {
        let d = std::env::temp_dir().join(format!("tm-ledger-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&d);
        d
    }

    #[test]
    fn dates_come_out_right() {
        assert_eq!(iso(0), "1970-01-01T00:00:00Z");
        assert_eq!(iso(951_782_400), "2000-02-29T00:00:00Z");
        assert_eq!(iso(1_000_000_000), "2001-09-09T01:46:40Z");
        assert_eq!(iso(1_793_041_603), "2026-10-26T19:06:43Z");
    }

    #[test]
    fn prices_follow_the_published_list() {
        let u = Usage {
            input: 1_000_000,
            cached: 400_000,
            output: 100_000,
        };
        // 600k fresh at 1.25 + 400k cached at 0.15 + 100k out at 4.25
        assert!((cost("muse-spark-1.3", u) - (0.75 + 0.06 + 0.425)).abs() < 1e-9);
        assert!((cost("muse-spark-1.3-contributor", u) - (0.06 + 0.0008 + 0.02)).abs() < 1e-9);
        assert_eq!(price("something-new"), price("muse-spark-1.3"));
    }

    #[test]
    fn the_cap_aborts_before_the_call_and_resets_next_day() {
        let d = dir("cap");
        let day1 = 1_793_041_603;
        {
            let mut l = Ledger::open_at(&d, day1).unwrap();
            let worst = l.preflight("muse-spark-1.3", 12_000, 6000, 0.05).unwrap();
            assert!(worst > 0.03 && worst < 0.05, "{worst}");
            l.record(
                "muse-spark-1.3",
                "completed",
                Charge::Usage(Usage {
                    input: 4000,
                    cached: 0,
                    output: 2000,
                }),
                "",
            )
            .unwrap();
            let e = l
                .preflight("muse-spark-1.3", 12_000, 6000, 0.05)
                .unwrap_err();
            assert!(e.contains("cap") && e.contains("Nothing was sent"), "{e}");
        }
        let mut next = Ledger::open_at(&d, day1 + 86_400).unwrap();
        assert_eq!(next.spent_today().unwrap(), 0.0);
        assert!(next.preflight("muse-spark-1.3", 12_000, 6000, 0.05).is_ok());
        let _ = fs::remove_dir_all(&d);
    }

    #[test]
    fn failed_and_estimated_responses_are_rows_too() {
        let d = dir("rows");
        let mut l = Ledger::open_at(&d, 1_793_041_603).unwrap();
        l.record("muse-spark-1.3", "http_429", Charge::Free, "retry-after 2")
            .unwrap();
        l.record("muse-spark-1.3", "dropped", Charge::Estimate(0.04), "")
            .unwrap();
        assert!((l.spent_today().unwrap() - 0.04).abs() < 1e-9);
        let text = fs::read_to_string(d.join("ledger.jsonl")).unwrap();
        assert_eq!(text.lines().count(), 2);
        assert!(text.contains("\"est\":true") && text.contains("http_429"));
        let _ = fs::remove_dir_all(&d);
    }

    #[test]
    fn a_torn_line_does_not_stop_the_count() {
        let d = dir("torn");
        fs::create_dir_all(&d).unwrap();
        fs::write(
            d.join("ledger.jsonl"),
            "{\"day\":\"2026-10-26\",\"usd\":0.5}\n{\"day\":\"2026-10-2",
        )
        .unwrap();
        let mut l = Ledger::open_at(&d, 1_793_041_603).unwrap();
        assert!((l.spent_today().unwrap() - 0.5).abs() < 1e-9);
        let _ = fs::remove_dir_all(&d);
    }
}
