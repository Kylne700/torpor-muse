//! Hiding secrets in text before it goes anywhere: passwords, tokens, key-like words and URL credentials.

const STRONG: [&str; 14] = [
    "secret",
    "token",
    "apikey",
    "api_key",
    "api-key",
    "private_key",
    "privatekey",
    "credential",
    "auth_key",
    "authkey",
    "signing",
    "salt",
    "authorization",
    "bearer",
];

const PASSWORDS: [&str; 3] = ["password", "passwd", "pwd"];

// Built with concat! so that this file holds no complete key-like literal.
const PREFIXES: [&str; 17] = [
    "sk-",
    "sk_live_",
    "sk_test_",
    "pk_live_",
    concat!("gh", "p_"),
    concat!("gh", "o_"),
    concat!("gh", "u_"),
    concat!("gh", "s_"),
    concat!("gh", "r_"),
    concat!("github", "_pat_"),
    concat!("gl", "pat-"),
    concat!("xo", "xb-"),
    concat!("xo", "xp-"),
    "AKIA",
    "AIza",
    "LLM_",
    "eyJ",
];

pub(in crate::suggest) const MASK: &str = "[redacted]";

fn is_tok(c: char) -> bool {
    c.is_ascii_alphanumeric() || matches!(c, '_' | '-' | '+' | '/' | '.')
}

/// Looks like a key: long and mixed, or a known key prefix with a digit and enough length.
fn key_like(t: &str) -> bool {
    let n = t.chars().count();
    let digits = t.chars().filter(char::is_ascii_digit).count();
    let letters = t.chars().filter(char::is_ascii_alphabetic).count();
    (n >= 24 && digits >= 2 && letters >= 8)
        || (digits >= 1
            && PREFIXES
                .iter()
                .any(|p| t.starts_with(p) && n >= p.len() + 8))
}

/// shortcut: a password literal is kept only in a `.synthetic` project, where the model needs the seed user's login.
/// Anywhere else it is masked like a secret. Upgrade when real projects need their seed logins (an opt-in setting).
fn sensitive(name: &str, synthetic: bool) -> bool {
    STRONG.iter().any(|k| name.contains(k))
        || (!synthetic && (PASSWORDS.iter().any(|k| name.contains(k)) || name == "pass"))
}

fn mask_url_credentials(s: &str) -> String {
    let (mut out, mut rest) = (String::new(), s);
    while let Some(i) = rest.find("://") {
        let (head, tail) = rest.split_at(i + 3);
        out.push_str(head);
        let end = tail
            .find(|c: char| matches!(c, '/' | '?' | '#' | '"' | '\'' | '`') || c.is_whitespace())
            .unwrap_or(tail.len());
        match tail[..end].rfind('@') {
            Some(at) if tail[..at].contains(':') => {
                out.push_str(MASK);
                rest = &tail[at..];
            }
            _ => rest = tail,
        }
    }
    out.push_str(rest);
    out
}

fn apply(c: &[char], cuts: &[(usize, usize)]) -> String {
    let (mut out, mut at) = (String::new(), 0);
    for &(a, b) in cuts {
        out.extend(&c[at..a]);
        out.push_str(MASK);
        at = b;
    }
    out.extend(&c[at..]);
    out
}

/// The value after `name =` / `name:` / `"name": ` is masked when the name is sensitive. A quoted value is masked
/// inside its quotes; a bare word of 6+ characters is masked; an expression (`a.b`, `f(x)`, `a[i]`) is kept.
fn mask_assignments(line: &str, synthetic: bool) -> String {
    let c: Vec<char> = line.chars().collect();
    let mut cuts: Vec<(usize, usize)> = Vec::new();
    let mut i = 0;
    while i < c.len() {
        let skip = !matches!(c[i], '=' | ':')
            || matches!(c.get(i + 1), Some('=' | '>' | '/'))
            || (i > 0 && matches!(c[i - 1], '=' | '!' | '<' | '>'));
        if skip {
            i += 1;
            continue;
        }
        let mut j = i;
        while j > 0 && c[j - 1] == ' ' {
            j -= 1;
        }
        if j > 0 && matches!(c[j - 1], '"' | '\'' | '`') {
            j -= 1;
        }
        let end = j;
        while j > 0 && (c[j - 1].is_ascii_alphanumeric() || matches!(c[j - 1], '_' | '-' | '.')) {
            j -= 1;
        }
        let name: String = c[j..end].iter().collect::<String>().to_lowercase();
        let mut v = i + 1;
        while v < c.len() && matches!(c[v], ' ' | '*') {
            v += 1;
        }
        if v >= c.len() || !sensitive(&name, synthetic) {
            i += 1;
            continue;
        }
        if matches!(c[v], '"' | '\'' | '`') {
            let close = c[v + 1..]
                .iter()
                .position(|&x| x == c[v])
                .map_or(c.len(), |p| v + 1 + p);
            if close > v + 1 {
                cuts.push((v + 1, close));
            }
            i = close.max(i + 1);
        } else {
            let mut e = v;
            while e < c.len()
                && (c[e].is_ascii_alphanumeric() || matches!(c[e], '_' | '-' | '+' | '/'))
            {
                e += 1;
            }
            let word: String = c[v..e].iter().collect::<String>().to_lowercase();
            let plain = ["true", "false", "null", "undefined", "none"].contains(&word.as_str());
            let expression = matches!(c.get(e), Some('.' | '(' | '['));
            if e - v >= 6 && !plain && !expression {
                cuts.push((v, e));
            }
            i = e.max(i + 1);
        }
    }
    apply(&c, &cuts)
}

/// The word after `Bearer ` or `Basic ` (an Authorization header).
fn mask_bearer(s: &str) -> String {
    let low = s.to_ascii_lowercase();
    let c: Vec<char> = s.chars().collect();
    let l: Vec<char> = low.chars().collect();
    let mut cuts = Vec::new();
    for word in ["bearer ", "basic "] {
        let w: Vec<char> = word.chars().collect();
        let mut i = 0;
        while i + w.len() < l.len() {
            if l[i..i + w.len()] == w[..] {
                let a = i + w.len();
                let mut e = a;
                while e < c.len() && (is_tok(c[e]) || c[e] == '=') {
                    e += 1;
                }
                if e - a >= 8 {
                    cuts.push((a, e));
                }
                i = e.max(i + 1);
            } else {
                i += 1;
            }
        }
    }
    cuts.sort_unstable();
    cuts.dedup();
    // overlapping cuts cannot happen: the two words never share the text after them
    apply(&c, &cuts)
}

fn mask_key_like(s: &str) -> String {
    let c: Vec<char> = s.chars().collect();
    let (mut cuts, mut i) = (Vec::new(), 0);
    while i < c.len() {
        if !is_tok(c[i]) {
            i += 1;
            continue;
        }
        let mut e = i;
        while e < c.len() && is_tok(c[e]) {
            e += 1;
        }
        let run: String = c[i..e].iter().collect();
        if key_like(&run) {
            cuts.push((i, e));
        }
        i = e;
    }
    apply(&c, &cuts)
}

/// Removes what looks like a secret from text that is about to leave the machine. Best effort by shape and by name:
/// PEM blocks, URL credentials, values of sensitive names, Bearer words and key-like tokens.
pub fn redact(text: &str, synthetic: bool) -> String {
    let (mut out, mut in_pem) = (String::new(), false);
    for line in text.lines() {
        if in_pem {
            in_pem = !line.contains("-----END ");
            continue;
        }
        if line.contains("-----BEGIN ") {
            in_pem = !line.contains("-----END ");
            out.push_str("[key block removed]\n");
            continue;
        }
        let l = mask_url_credentials(line);
        let l = mask_assignments(&l, synthetic);
        let l = mask_bearer(&l);
        out.push_str(&mask_key_like(&l));
        out.push('\n');
    }
    out
}
