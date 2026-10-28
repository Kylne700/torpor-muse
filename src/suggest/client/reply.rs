//! What a response means: the text and usage of a good one, or a named failure with its charge.

use super::http::{Net, Reply, clean};
use crate::ledger::{Charge, Usage};
use serde_json::Value;

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

    /// The kind the hook tells the user about when a run ends on this fault: `key` (Meta refused the key or its
    /// billing), `api` (the call itself failed). "" = the model's own fault: the run ends `rejected` (see `work`).
    pub fn code(&self) -> &'static str {
        match self {
            Bad::Billing(_) | Bad::Auth(_) => "key",
            _ if self.rejected() => "",
            _ => "api",
        }
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

pub(super) fn error_message(v: &Option<Value>) -> String {
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
    fn only_a_fault_around_the_model_has_a_kind_the_user_is_told() {
        let code = |s: u16| {
            interpret(&reply(s, r#"{"error":{"message":"m"}}"#), 0.05)
                .outcome
                .unwrap_err()
                .code()
        };
        assert_eq!((code(401), code(402)), ("key", "key"));
        assert_eq!(
            (code(400), code(404), code(429), code(500)),
            ("api", "api", "api", "api")
        );
        assert_eq!(
            (Bad::Network("n".into()).code(), Bad::Timeout.code()),
            ("api", "api")
        );
        for model_fault in [
            Bad::Truncated,
            Bad::Empty,
            Bad::Refused("r".into()),
            Bad::Malformed("m".into()),
        ] {
            assert_eq!(model_fault.code(), "", "{model_fault:?}");
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
}
