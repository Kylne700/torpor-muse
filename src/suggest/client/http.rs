//! The address of the API and the one HTTP call to it, with the reply or the net failure as plain values.

use super::config::Key;
use crate::suggest::state::env;
use std::time::Duration;

pub const DEFAULT_BASE: &str = "https://api.meta.ai";

const MAX_BODY: u64 = 2_000_000;

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

pub struct Reply {
    pub status: u16,
    pub retry_after: Option<String>,
    pub body: String,
}

pub enum Net {
    Timeout,
    Other(String),
}

pub(super) fn net(e: ureq::Error) -> Net {
    match e {
        ureq::Error::Timeout(_) => Net::Timeout,
        other => Net::Other(clean(&other.to_string(), 160)),
    }
}

/// One POST to `/v1/responses`.
pub fn post(base: &Base, key: &Key, body: &str, timeout: Duration) -> Result<Reply, Net> {
    let json = [("Content-Type", "application/json")];
    post_to(
        base,
        "/v1/responses",
        key,
        &json,
        body.as_bytes(),
        MAX_BODY,
        timeout,
    )
}

/// One POST to `path` under `base`, with the key and `headers`. Redirects are not followed (the key must not travel
/// on), and a status of 4xx or 5xx is a reply, not an error, so that its body and Retry-After can be read. A body
/// longer than `limit` bytes is an error.
pub(in crate::suggest) fn post_to(
    base: &Base,
    path: &str,
    key: &Key,
    headers: &[(&str, &str)],
    body: &[u8],
    limit: u64,
    timeout: Duration,
) -> Result<Reply, Net> {
    let agent: ureq::Agent = ureq::Agent::config_builder()
        .http_status_as_error(false)
        .max_redirects(0)
        .timeout_global(Some(timeout))
        .timeout_connect(Some(timeout.min(Duration::from_secs(10))))
        .user_agent(concat!("torpor-muse/", env!("CARGO_PKG_VERSION")))
        .build()
        .into();
    let mut req = agent
        .post(format!("{}{path}", base.url))
        .header("Authorization", key.bearer());
    for (name, value) in headers {
        req = req.header(*name, *value);
    }
    let mut resp = req.send(body).map_err(net)?;
    let status = resp.status().as_u16();
    let retry_after = resp
        .headers()
        .get("retry-after")
        .and_then(|v| v.to_str().ok())
        .map(String::from);
    let body = resp
        .body_mut()
        .with_config()
        .limit(limit)
        .read_to_string()
        .map_err(net)?;
    Ok(Reply {
        status,
        retry_after,
        body,
    })
}

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

#[cfg(test)]
mod tests {
    use super::*;

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
}
