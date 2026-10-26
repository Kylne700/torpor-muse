//! The Meta API side: where the key comes from, which address is allowed, the one HTTP call, and what a response
//! means. The key is never printed, logged or put on a command line. Owner: S3b-writer.
use super::state::env;
use crate::ledger::{Charge, Usage};
use serde_json::Value;
use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::time::Duration;

pub const DEFAULT_BASE: &str = "https://api.meta.ai";
const MAX_BODY: u64 = 2_000_000;

// ---------------------------------------------------------------- key and config

/// The API key. It has no Display, and Debug hides it.
#[derive(Clone)]
pub struct Key(String);

impl std::fmt::Debug for Key {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Key(hidden)")
    }
}

impl Key {
    fn new(s: &str) -> Result<Self, String> {
        let s = s.trim();
        if s.is_empty() || s.len() > 400 || s.chars().any(|c| c.is_whitespace() || c.is_control()) {
            return Err("the API key is empty or not one line of plain text".into());
        }
        Ok(Self(s.to_string()))
    }

    /// Replaces the key in text that is about to leave this process or be shown (belt and braces).
    pub fn scrub(&self, s: &str) -> String {
        s.replace(&self.0, "[key]")
    }

    fn bearer(&self) -> String {
        format!("Bearer {}", self.0)
    }
}

#[derive(Default, Debug, PartialEq)]
pub struct Config {
    pub key_file: Option<PathBuf>,
    pub day_cap: Option<f64>,
}

pub fn config_path() -> Option<PathBuf> {
    env("HOME").map(|h| Path::new(&h).join(".config/torpor-muse/config"))
}

/// A file with a secret or a path to one must not be readable by group or others.
fn check_private(p: &Path, what: &str) -> Result<(), String> {
    let m = fs::metadata(p).map_err(|e| format!("cannot read {what} ({}): {e}", p.display()))?;
    if !m.is_file() || m.permissions().mode() & 0o077 != 0 {
        return Err(format!(
            "{what} ({}) must be a plain file that only you can read: run chmod 600 on it",
            p.display()
        ));
    }
    Ok(())
}

pub fn parse_config(text: &str) -> Result<Config, String> {
    let mut c = Config::default();
    for (n, line) in text.lines().enumerate() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let (k, v) = line
            .split_once('=')
            .ok_or_else(|| format!("config line {}: expected `name = value`", n + 1))?;
        let v = v.trim().trim_matches('"');
        match k.trim() {
            "key_file" if !v.is_empty() => c.key_file = Some(PathBuf::from(v)),
            "day_cap_usd" => {
                c.day_cap = Some(
                    v.parse::<f64>()
                        .ok()
                        .filter(|x| x.is_finite() && *x >= 0.0)
                        .ok_or_else(|| {
                            format!("config line {}: day_cap_usd must be a number", n + 1)
                        })?,
                );
            }
            other => {
                return Err(format!(
                    "config line {}: unknown or empty setting `{other}`",
                    n + 1
                ));
            }
        }
    }
    Ok(c)
}

/// `~/.config/torpor-muse/config` (mode 0600, refused otherwise): `key_file = <path>`, `day_cap_usd = <number>`.
/// It holds the path of the key file, never the key.
pub fn load_config() -> Result<Config, String> {
    let Some(p) = config_path().filter(|p| p.exists()) else {
        return Ok(Config::default());
    };
    check_private(&p, "the config file")?;
    let text = fs::read_to_string(&p).map_err(|e| format!("cannot read the config file: {e}"))?;
    parse_config(&text)
}

/// For the trigger: is there a key source a detached run could use (it gets no key in its environment)?
pub fn key_source_ok(cfg: &Config) -> bool {
    cfg.key_file
        .as_deref()
        .is_some_and(|f| check_private(f, "the key file").is_ok())
}

/// `META_API_KEY` when it is set (run by hand); otherwise the file the config names. Never from argv. A run started
/// by the trigger gets no `META_API_KEY`, so it always takes the file.
pub fn load_key(cfg: &Config) -> Result<Key, String> {
    key_from(cfg, env("META_API_KEY"))
}

fn key_from(cfg: &Config, from_env: Option<String>) -> Result<Key, String> {
    if let Some(k) = from_env {
        return Key::new(&k);
    }
    let f = cfg.key_file.as_deref().ok_or(
        "no API key: set META_API_KEY, or put `key_file = <path of a file holding the key>` in ~/.config/torpor-muse/config",
    )?;
    check_private(f, "the key file")?;
    Key::new(&fs::read_to_string(f).map_err(|e| format!("cannot read the key file: {e}"))?)
}

pub fn day_cap(cfg: &Config) -> Result<f64, String> {
    match env("TORPOR_MUSE_DAY_CAP_USD") {
        Some(v) => v
            .parse::<f64>()
            .ok()
            .filter(|x| x.is_finite() && *x >= 0.0)
            .ok_or_else(|| "TORPOR_MUSE_DAY_CAP_USD must be a number".into()),
        None => Ok(cfg.day_cap.unwrap_or(crate::ledger::DEFAULT_CAP_USD)),
    }
}

// ---------------------------------------------------------------- address

pub struct Base {
    pub url: String,
    /// The address is `http://127.0.0.1`: a test run. Only then are the short test timeouts honoured.
    pub loopback: bool,
}

/// `TORPOR_MUSE_API_BASE` is for tests: refused unless https, or http on 127.0.0.1.
pub fn api_base() -> Result<Base, String> {
    let Some(b) = env("TORPOR_MUSE_API_BASE") else {
        return Ok(Base {
            url: DEFAULT_BASE.into(),
            loopback: false,
        });
    };
    parse_base(&b)
}

pub fn parse_base(b: &str) -> Result<Base, String> {
    let bad = || {
        format!("TORPOR_MUSE_API_BASE must be https://... or http://127.0.0.1[:port], got `{b}`")
    };
    if b.chars()
        .any(|c| c.is_whitespace() || c.is_control() || matches!(c, '@' | '?' | '#' | '\\'))
    {
        return Err(bad());
    }
    let b = b.trim_end_matches('/');
    if let Some(host) = b.strip_prefix("https://") {
        return if host.is_empty() || host.starts_with(':') {
            Err(bad())
        } else {
            Ok(Base {
                url: b.into(),
                loopback: false,
            })
        };
    }
    let rest = b.strip_prefix("http://127.0.0.1").ok_or_else(bad)?;
    let port_ok = rest.is_empty()
        || rest
            .strip_prefix(':')
            .is_some_and(|p| !p.is_empty() && p.chars().all(|c| c.is_ascii_digit()));
    if !port_ok {
        return Err(bad());
    }
    Ok(Base {
        url: b.into(),
        loopback: true,
    })
}

// ---------------------------------------------------------------- the call

pub struct Reply {
    pub status: u16,
    pub retry_after: Option<String>,
    pub body: String,
}

pub enum Net {
    Timeout,
    Other(String),
}

fn net(e: ureq::Error) -> Net {
    match e {
        ureq::Error::Timeout(_) => Net::Timeout,
        other => Net::Other(clean(&other.to_string(), 160)),
    }
}

/// One POST to `/v1/responses`. Redirects are not followed (the key must not travel on), and a status of 4xx or 5xx
/// is a reply, not an error, so that its body and Retry-After can be read.
pub fn post(base: &Base, key: &Key, body: &str, timeout: Duration) -> Result<Reply, Net> {
    let agent: ureq::Agent = ureq::Agent::config_builder()
        .http_status_as_error(false)
        .max_redirects(0)
        .timeout_global(Some(timeout))
        .timeout_connect(Some(timeout.min(Duration::from_secs(10))))
        .user_agent(concat!("torpor-muse/", env!("CARGO_PKG_VERSION")))
        .build()
        .into();
    let mut resp = agent
        .post(format!("{}/v1/responses", base.url))
        .header("Authorization", key.bearer())
        .header("Content-Type", "application/json")
        .send(body)
        .map_err(net)?;
    let status = resp.status().as_u16();
    let retry_after = resp
        .headers()
        .get("retry-after")
        .and_then(|v| v.to_str().ok())
        .map(String::from);
    let body = resp
        .body_mut()
        .with_config()
        .limit(MAX_BODY)
        .read_to_string()
        .map_err(net)?;
    Ok(Reply {
        status,
        retry_after,
        body,
    })
}

// ---------------------------------------------------------------- what a response means

/// Text from outside, safe to show: control characters become spaces, at most `max` characters.
pub fn clean(s: &str, max: usize) -> String {
    let flat: String = s
        .chars()
        .map(|c| if c.is_control() { ' ' } else { c })
        .collect();
    flat.split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .chars()
        .take(max)
        .collect()
}

#[derive(Debug, PartialEq)]
pub enum Bad {
    Billing(String),
    Auth(u16),
    Request(String),
    Other(u16),
    RateLimited { wait: Option<u64>, msg: String },
    Server(String),
    Network(String),
    Timeout,
    Truncated,
    Refused(String),
    Empty,
    Malformed(String),
}

impl Bad {
    /// One more try may help: the fault is on the way (server, network) or in this one answer.
    pub fn retryable(&self) -> bool {
        !matches!(
            self,
            Bad::Billing(_) | Bad::Auth(_) | Bad::Request(_) | Bad::Other(_)
        )
    }

    /// A fault in what the model wrote (not in the world around it): the run ends as "rejected".
    pub fn rejected(&self) -> bool {
        matches!(
            self,
            Bad::Truncated | Bad::Refused(_) | Bad::Empty | Bad::Malformed(_)
        )
    }

    pub fn reason(&self) -> String {
        match self {
            Bad::Billing(c) => format!(
                "Meta says billing is not set up for this key ({c}). Nothing was saved. Set up billing for the key, then ask again."
            ),
            Bad::Auth(s) => format!("Meta refused the API key (HTTP {s}). Check the key file."),
            Bad::Request(m) => format!("Meta rejected the request: {m}"),
            Bad::Other(s) => format!("unexpected HTTP status {s} from the Meta API"),
            Bad::RateLimited { wait, msg } => match wait {
                Some(w) => format!("rate limited by Meta (retry after {w} s): {msg}"),
                None => format!("rate limited by Meta: {msg}"),
            },
            Bad::Server(m) => format!("the Meta API had a server error: {m}"),
            Bad::Network(m) => format!("could not get a whole answer from the Meta API: {m}"),
            Bad::Timeout => "the Meta API did not answer in time".into(),
            Bad::Truncated => "the model ran out of room before it finished its answer".into(),
            Bad::Refused(m) => format!("the model refused to answer: {m}"),
            Bad::Empty => "the model gave no answer text".into(),
            Bad::Malformed(m) => m.clone(),
        }
    }
}

/// One response, read: what to write in the ledger and what came out of it.
pub struct Reading {
    pub status: String,
    pub charge: Charge,
    pub outcome: Result<String, Bad>,
}

pub fn from_net(n: Net, worst: f64) -> Reading {
    let (status, bad) = match n {
        Net::Timeout => ("timeout", Bad::Timeout),
        Net::Other(m) => ("network", Bad::Network(m)),
    };
    // The call may have been billed although we saw no usage: charge the worst case.
    Reading {
        status: status.into(),
        charge: Charge::Estimate(worst),
        outcome: Err(bad),
    }
}

fn usage(v: &Value) -> Option<Usage> {
    let u = v.get("usage")?;
    Some(Usage {
        input: u["input_tokens"].as_u64()?,
        cached: u["input_tokens_details"]["cached_tokens"]
            .as_u64()
            .unwrap_or(0),
        output: u["output_tokens"].as_u64()?,
    })
}

fn error_message(v: &Option<Value>) -> String {
    v.as_ref()
        .and_then(|v| v["error"]["message"].as_str())
        .map_or_else(|| "(no message)".into(), |m| clean(m, 200))
}

pub fn interpret(r: &Reply, worst: f64) -> Reading {
    let v: Option<Value> = serde_json::from_str(&r.body).ok();
    let failed = |bad: Bad, charge: Charge| Reading {
        status: format!("http_{}", r.status),
        charge,
        outcome: Err(bad),
    };
    match r.status {
        200 => {}
        402 => {
            let code = v
                .as_ref()
                .and_then(|v| v["error"]["code"].as_str())
                .map_or("billing", |c| c);
            return failed(Bad::Billing(clean(code, 60)), Charge::Free);
        }
        401 | 403 => return failed(Bad::Auth(r.status), Charge::Free),
        400 => return failed(Bad::Request(error_message(&v)), Charge::Free),
        429 => {
            let wait = r
                .retry_after
                .as_deref()
                .and_then(|s| s.trim().parse::<u64>().ok());
            return failed(
                Bad::RateLimited {
                    wait,
                    msg: error_message(&v),
                },
                Charge::Free,
            );
        }
        // The model may have run before the server failed: charge the worst case so the cap stays safe.
        500..=599 => return failed(Bad::Server(error_message(&v)), Charge::Estimate(worst)),
        s => return failed(Bad::Other(s), Charge::Free),
    }
    let ok = |status: &str, charge: Charge, outcome: Result<String, Bad>| Reading {
        status: status.into(),
        charge,
        outcome,
    };
    let Some(v) = v else {
        let bad = Bad::Malformed("the response of the Meta API is not JSON".into());
        return ok("completed", Charge::Estimate(worst), Err(bad));
    };
    let charge = usage(&v).map_or(Charge::Estimate(worst), Charge::Usage);
    let status = v["status"].as_str().unwrap_or("unknown");
    if status == "incomplete" {
        return ok(status, charge, Err(Bad::Truncated));
    }
    if status != "completed" || !v["error"].is_null() {
        let msg = format!(
            "response status {status}: {}",
            error_message(&Some(v.clone()))
        );
        return ok(status, charge, Err(Bad::Server(msg)));
    }
    let (mut text, mut refusal) = (String::new(), None);
    for item in v["output"]
        .as_array()
        .into_iter()
        .flatten()
        .filter(|i| i["type"] == "message")
    {
        for part in item["content"].as_array().into_iter().flatten() {
            match part["type"].as_str() {
                Some("output_text") => text.push_str(part["text"].as_str().unwrap_or("")),
                Some("refusal") => {
                    refusal = Some(clean(part["refusal"].as_str().unwrap_or(""), 200))
                }
                _ => {}
            }
        }
    }
    let outcome = match refusal {
        Some(m) => Err(Bad::Refused(m)),
        None if text.trim().is_empty() => Err(Bad::Empty),
        None => Ok(text),
    };
    ok(status, charge, outcome)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn reply(status: u16, body: &str) -> Reply {
        Reply {
            status,
            retry_after: None,
            body: body.into(),
        }
    }

    fn done(text: &str) -> String {
        serde_json::json!({
            "status": "completed",
            "usage": {"input_tokens": 1000, "input_tokens_details": {"cached_tokens": 200}, "output_tokens": 500},
            "output": [{"type": "reasoning"}, {"type": "message", "content": [{"type": "output_text", "text": text}]}]
        })
        .to_string()
    }

    #[test]
    fn a_good_response_gives_text_and_usage() {
        let r = interpret(&reply(200, &done("{\"a\":1}")), 0.05);
        assert_eq!(r.outcome.unwrap(), "{\"a\":1}");
        match r.charge {
            Charge::Usage(u) => assert_eq!((u.input, u.cached, u.output), (1000, 200, 500)),
            c => panic!("{c:?}"),
        }
    }

    #[test]
    fn every_fault_is_a_named_failure_with_a_charge() {
        let e = |s: u16| {
            interpret(
                &reply(
                    s,
                    r#"{"error":{"message":"m","code":"billing_not_configured"}}"#,
                ),
                0.05,
            )
        };
        assert_eq!(
            e(402).outcome.unwrap_err(),
            Bad::Billing("billing_not_configured".into())
        );
        assert!(matches!(e(400).outcome, Err(Bad::Request(_))));
        assert!(matches!(e(401).outcome, Err(Bad::Auth(401))));
        assert!(matches!(e(500).outcome, Err(Bad::Server(_))));
        assert!(matches!(e(404).outcome, Err(Bad::Other(404))));
        assert!(matches!(e(402).charge, Charge::Free));
        assert!(
            matches!(e(500).charge, Charge::Estimate(_)),
            "a server fault may still have been billed"
        );
        let mut r = reply(429, r#"{"error":{"message":"slow down"}}"#);
        r.retry_after = Some("2".into());
        assert_eq!(
            interpret(&r, 0.05).outcome.unwrap_err(),
            Bad::RateLimited {
                wait: Some(2),
                msg: "slow down".into()
            }
        );
        let truncated = r#"{"status":"incomplete","incomplete_details":{"reason":"max_output_tokens"},"usage":{"input_tokens":10,"output_tokens":20},"output":[]}"#;
        let t = interpret(&reply(200, truncated), 0.05);
        assert_eq!(t.outcome.unwrap_err(), Bad::Truncated);
        assert!(
            matches!(t.charge, Charge::Usage(_)),
            "a cut answer is billed"
        );
        let refusal = r#"{"status":"completed","usage":{"input_tokens":1,"output_tokens":1},"output":[{"type":"message","content":[{"type":"refusal","refusal":"no"}]}]}"#;
        assert_eq!(
            interpret(&reply(200, refusal), 0.05).outcome.unwrap_err(),
            Bad::Refused("no".into())
        );
        let empty = r#"{"status":"completed","usage":{"input_tokens":1,"output_tokens":1},"output":[{"type":"reasoning"}]}"#;
        assert_eq!(
            interpret(&reply(200, empty), 0.05).outcome.unwrap_err(),
            Bad::Empty
        );
        assert!(matches!(
            interpret(&reply(200, "<html>"), 0.05).outcome,
            Err(Bad::Malformed(_))
        ));
        assert!(matches!(
            interpret(&reply(200, "<html>"), 0.05).charge,
            Charge::Estimate(_)
        ));
    }

    #[test]
    fn only_the_right_faults_are_retried_or_called_rejected() {
        assert!(
            Bad::Timeout.retryable()
                && Bad::Truncated.retryable()
                && Bad::Server("x".into()).retryable()
        );
        assert!(!Bad::Billing("x".into()).retryable() && !Bad::Request("x".into()).retryable());
        assert!(Bad::Empty.rejected() && Bad::Malformed("x".into()).rejected());
        assert!(!Bad::Timeout.rejected() && !Bad::Billing("x".into()).rejected());
    }

    #[test]
    fn the_address_rule() {
        assert!(parse_base("https://api.meta.ai").is_ok());
        assert!(!parse_base("https://api.meta.ai/").unwrap().loopback);
        let l = parse_base("http://127.0.0.1:8123").unwrap();
        assert!(l.loopback && l.url == "http://127.0.0.1:8123");
        assert!(parse_base("http://127.0.0.1").is_ok());
        for bad in [
            "http://api.meta.ai",
            "http://127.0.0.1.evil.test",
            "http://127.0.0.1:80x",
            "http://localhost:80",
            "ftp://127.0.0.1",
            "https://",
            "https://user:pw@host",
            "https://host/?a=b",
            "https://ho st",
            "",
        ] {
            assert!(parse_base(bad).is_err(), "{bad} must be refused");
        }
    }

    #[test]
    fn the_config_names_a_file_and_nothing_else() {
        let c = parse_config("# a comment\nkey_file = /tmp/k\nday_cap_usd = 1.5\n").unwrap();
        assert_eq!(c.key_file, Some(PathBuf::from("/tmp/k")));
        assert_eq!(c.day_cap, Some(1.5));
        assert!(
            parse_config("api_key = abc").is_err(),
            "a key in the config is refused"
        );
        assert!(parse_config("day_cap_usd = lots").is_err());
        assert!(parse_config("key_file").is_err());
    }

    #[test]
    fn files_that_others_can_read_are_refused_and_the_key_stays_hidden() {
        let d = std::env::temp_dir().join(format!("tm-client-{}", std::process::id()));
        let _ = fs::remove_dir_all(&d);
        fs::create_dir_all(&d).unwrap();
        let f = d.join("k");
        fs::write(&f, "LLM_testkey0123456789\n").unwrap();
        fs::set_permissions(&f, fs::Permissions::from_mode(0o644)).unwrap();
        let cfg = Config {
            key_file: Some(f.clone()),
            day_cap: None,
        };
        assert!(
            key_from(&cfg, None)
                .unwrap_err()
                .contains("only you can read")
        );
        assert!(!key_source_ok(&cfg));
        fs::set_permissions(&f, fs::Permissions::from_mode(0o600)).unwrap();
        let key = key_from(&cfg, None).unwrap();
        assert!(key_source_ok(&cfg));
        assert_eq!(format!("{key:?}"), "Key(hidden)");
        assert_eq!(key.scrub("a LLM_testkey0123456789 b"), "a [key] b");
        assert!(Key::new("two words").is_err() && Key::new("").is_err());
        let _ = fs::remove_dir_all(&d);
    }
}
