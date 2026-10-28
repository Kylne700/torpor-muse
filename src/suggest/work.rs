//! The work of a run: ask the model, wait out a rate limit, and turn the answer into a saved check or an outcome.

use super::client::Bad;
use super::schema::Start;
use super::types::{Ctx, Kind, Outcome, Plan};
use super::{attach, client, prompt, schema, state};
use serde_json::{Value, json};
use std::time::{Duration, Instant};

/// Low reasoning effort still spends about 500 to 1000 tokens before the answer, so the budget is large.
const MAX_OUT: u32 = 6000;

const CACHE_KEY: &str = concat!("torpor-muse-writer-", env!("CARGO_PKG_VERSION"));

const RETRY_WAIT_MAX_S: u64 = 20;

const RETRY_WAIT_DEFAULT_S: u64 = 2;

const MIN_TIME_FOR_RETRY: Duration = Duration::from_secs(90);

/// How long to wait before the one retry, or `None` when the wait would pass the budget.
fn backoff(bad: &Bad) -> Option<Duration> {
    match bad {
        Bad::RateLimited { wait, .. } => {
            let w = wait.unwrap_or(RETRY_WAIT_DEFAULT_S);
            (w <= RETRY_WAIT_MAX_S).then(|| Duration::from_secs(w))
        }
        _ => Some(Duration::ZERO),
    }
}

fn request(model: &str, input: &str, starts: &[Start]) -> Value {
    json!({
        "model": model,
        "instructions": prompt::INSTRUCTIONS,
        "input": input,
        "reasoning": {"effort": "low"},
        "max_output_tokens": MAX_OUT,
        "prompt_cache_key": CACHE_KEY,
        "text": {"format": schema::response_format(starts)},
    })
}

/// One call to the model per try, at most two tries, and the engine's verdict on the check.
pub(super) fn work(c: &mut Ctx, p: &Plan) -> Outcome {
    let mut retry: Option<(String, String)> = None;
    for attempt in 0..2u32 {
        let last = attempt == 1;
        let nonce = prompt::nonce();
        let rejected = retry.as_ref().map(|(a, m)| prompt::Rejected {
            answer: a,
            message: m,
        });
        let input = prompt::input(&nonce, &p.starts, &p.excerpt, &p.words, rejected.as_ref());
        let body = request(&p.model, &input, &p.starts).to_string();
        let reading = match ask(c, p, &body, attempt) {
            Ok(r) => r,
            Err(o) => return o,
        };
        let text = match reading.outcome {
            Ok(t) => t,
            Err(bad) => {
                let wait = backoff(&bad)
                    .filter(|_| bad.retryable() && !last && c.time_left() > MIN_TIME_FOR_RETRY);
                let Some(wait) = wait else {
                    let (kind, code) = if bad.rejected() {
                        (Kind::Rejected, "rejected")
                    } else {
                        (Kind::Advisory, bad.code())
                    };
                    return Outcome::new(kind, bad.reason()).code(code);
                };
                std::thread::sleep(wait);
                retry = None;
                continue;
            }
        };
        let mut answer = match schema::parse(&text) {
            Ok(a) => a,
            Err(e) if last => return Outcome::new(Kind::Rejected, e).code("rejected"),
            Err(e) => {
                retry = Some((c.safe(&text, 3000), e));
                continue;
            }
        };
        match answer.verdict {
            schema::Verdict::NotAClaim => {
                let why = c.safe(&answer.reason, 300);
                return Outcome::new(
                    Kind::Advisory,
                    format!("nothing to check: not a statement that something works ({why})"),
                );
            }
            schema::Verdict::CannotCheck => {
                let why = c.safe(&answer.reason, 300);
                return Outcome::new(
                    Kind::Advisory,
                    format!("cannot check this with HTTP requests ({why})"),
                );
            }
            schema::Verdict::Check => {}
        }
        // what the steps cannot prove, in the model's words (for a `check` the reason is only that)
        let mut gaps = vec![c.safe(&answer.reason, 200)];
        let gone = schema::drop_data_records(&mut answer, &p.excerpt);
        if !gone.is_empty() {
            let why = format!(
                "{} left out: it needs a record already in your data, which the check did not create",
                gone.join(", ")
            );
            let why = c.safe(&why, 300);
            if answer.steps.is_empty() {
                return Outcome::new(Kind::Advisory, format!("no check saved: {why}"));
            }
            gaps.push(why);
        }
        if schema::logout_refusal(&answer) {
            gaps.push(schema::LOGOUT_GAP.into());
        }
        gaps.retain(|g| !g.is_empty());
        let gap = gaps.join("; ");
        // `torpor promise show` prints the why (the engine takes 200 characters); the model's own words are never cut
        let room = 200usize.saturating_sub(answer.why.chars().count() + " Not covered: ".len());
        if !gap.is_empty() && room >= 20 {
            let short = if gap.chars().count() > room {
                format!("{}...", c.safe(&gap, room - 3))
            } else {
                gap.clone()
            };
            answer.why = format!("{} Not covered: {short}", answer.why);
        }
        let check = match schema::to_check(&answer, &p.starts) {
            Ok(v) => v,
            Err(e) if last => return Outcome::new(Kind::Rejected, e).code("rejected"),
            Err(e) => {
                retry = Some((c.safe(&text, 3000), e));
                continue;
            }
        };
        let said = match attach::add(
            &p.root,
            &check,
            c.time_left().saturating_sub(Duration::from_secs(5)),
        ) {
            Ok(a) => a,
            Err(e) => return Outcome::new(Kind::Error, e),
        };
        let turned_down = match said {
            attach::Added::Proven { id, line } => {
                return saved(c, p, Kind::Proven, id, &line, &gap);
            }
            attach::Added::Advisory { id, line } => {
                return saved(c, p, Kind::Advisory, id, &line, &gap);
            }
            attach::Added::Lint(m) => m,
            attach::Added::NotSaved { why, retry: true } => why,
            attach::Added::NotSaved { why, retry: false } => {
                return Outcome::new(
                    Kind::Advisory,
                    format!("Torpor did not save the check: {why}"),
                );
            }
            attach::Added::Failed(m) => return Outcome::new(Kind::Advisory, m),
        };
        if last || c.time_left() < MIN_TIME_FOR_RETRY {
            return Outcome::new(
                Kind::Advisory,
                format!("the engine turned the check down: {turned_down}"),
            );
        }
        retry = Some((c.safe(&text, 3000), turned_down));
    }
    Outcome::new(Kind::Advisory, "no check was made")
}

/// A saved check. What it does not cover also goes to `gaps.json`, for the hook to tell the user and the model.
fn saved(c: &Ctx, p: &Plan, kind: Kind, promise: Option<u32>, line: &str, gap: &str) -> Outcome {
    if let (Some(id), false) = (promise, gap.is_empty()) {
        let g = state::Gap {
            promise: id,
            session: c.session.clone(),
            gap: gap.into(),
        };
        if let Err(e) = state::add_gap(&p.root, g) {
            eprintln!("torpor-muse suggest: {e}");
        }
    }
    let message = if gap.is_empty() {
        line.to_string()
    } else {
        format!("{line} Not covered by this check: {gap}")
    };
    Outcome {
        kind,
        saved: true,
        promise,
        message,
        code: "",
    }
}

/// One response from the model, put into the ledger whatever it was, under the ledger's lock from the cap check
/// to the row (so two runs cannot both pass the cap).
fn ask(c: &mut Ctx, p: &Plan, body: &str, attempt: u32) -> Result<client::Reading, Outcome> {
    let err = |m: String| Outcome::new(Kind::Error, m);
    let mut ledger = crate::ledger::Ledger::open(&p.ledger_dir).map_err(err)?;
    let worst = ledger
        .preflight(&p.model, body.len(), MAX_OUT, p.cap)
        .map_err(|m| Outcome::new(Kind::Advisory, m).code("cap"))?;
    c.calls += 1;
    let timeout = p.http.min(c.time_left());
    let sent = Instant::now();
    let reading = match client::post(&p.base, &p.key, body, timeout) {
        Ok(reply) => client::interpret(&reply, worst),
        Err(n) => client::from_net(n, worst),
    };
    // the call's own time goes into the note, so the ledger shows how long the model took
    let secs = sent.elapsed().as_secs_f64();
    let note = match &reading.outcome {
        Ok(_) => format!("attempt {}, {secs:.1}s", attempt + 1),
        Err(bad) => format!(
            "attempt {}, {secs:.1}s: {}",
            attempt + 1,
            c.safe(&bad.reason(), 120)
        ),
    };
    // M2 evidence: the names in the FILE LIST that went out (names only, never what is in the files).
    // shortcut: names past 2000 characters are not recorded; upgrade when a ledger must show a bigger project's list.
    let names: Vec<&str> = p
        .excerpt
        .lines()
        .skip(1)
        .take_while(|l| !l.is_empty())
        .collect();
    let note = format!("{note}; sent: {}", c.safe(&names.join(", "), 2000));
    c.usd += ledger
        .record(&p.model, &reading.status, reading.charge, &note)
        .map_err(err)?;
    Ok(reading)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_rate_limit_wait_follows_retry_after_within_the_budget() {
        let limited = |w| Bad::RateLimited {
            wait: w,
            msg: String::new(),
        };
        assert_eq!(backoff(&limited(Some(2))), Some(Duration::from_secs(2)));
        assert_eq!(
            backoff(&limited(None)),
            Some(Duration::from_secs(RETRY_WAIT_DEFAULT_S))
        );
        assert_eq!(
            backoff(&limited(Some(RETRY_WAIT_MAX_S))),
            Some(Duration::from_secs(RETRY_WAIT_MAX_S))
        );
        assert_eq!(backoff(&limited(Some(RETRY_WAIT_MAX_S + 1))), None);
        assert_eq!(backoff(&Bad::Timeout), Some(Duration::ZERO));
    }

    #[test]
    fn the_request_asks_for_low_effort_a_cache_key_and_a_strict_schema() {
        let starts = vec![Start {
            key: "start".into(),
            cmd: "npm start --silent".into(),
        }];
        let r = request("muse-spark-1.3", "data", &starts);
        assert_eq!(r["reasoning"]["effort"], "low");
        assert_eq!(r["text"]["format"]["type"], "json_schema");
        assert_eq!(r["text"]["format"]["strict"], true);
        assert!(
            r["prompt_cache_key"]
                .as_str()
                .unwrap()
                .starts_with("torpor-muse-writer-")
        );
        assert_eq!(r["max_output_tokens"], MAX_OUT);
        assert!(r.get("tools").is_none() && r.get("start_cmd").is_none());
    }
}
