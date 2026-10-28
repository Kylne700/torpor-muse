//! The plan of a run: the local checks of the folder, the model to ask and the words that go into the request.

use super::schema::Start;
use super::state::env;
use super::types::{Ctx, Kind, Outcome, Plan};
use super::{DEFAULT_MODEL, attach, client, repo, state};
use std::path::Path;
use std::time::Duration;

const WORDS_MAX: usize = 1500;

/// A long prompt keeps its start and its end.
pub(super) fn cap_words(s: &str) -> String {
    let s = s.trim();
    let n = s.chars().count();
    if n <= WORDS_MAX {
        return s.to_string();
    }
    let head: String = s.chars().take(WORDS_MAX * 2 / 3).collect();
    let tail: String = s.chars().skip(n - WORDS_MAX / 3).collect();
    format!("{head} [...] {tail}")
}

pub(super) fn model_name() -> Result<String, String> {
    let m = env("TORPOR_MUSE_MODEL").unwrap_or_else(|| DEFAULT_MODEL.into());
    let plain = m.len() <= 64
        && m.chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | '_'));
    if plain {
        Ok(m)
    } else {
        Err("TORPOR_MUSE_MODEL is not a model id".into())
    }
}

/// The checks that cost nothing and need no key: a script to start the app with, and the engine. `say` runs them
/// before it uploads a clip, so a folder the writer would refuse never costs a transcription.
pub(super) fn local_checks(root: &Path) -> Result<Vec<Start>, Outcome> {
    let starts = repo::start_choices(root);
    if starts.is_empty() {
        return Err(Outcome::new(
            Kind::Advisory,
            "cannot make a check: package.json has no start, dev, serve or server script to start the app with",
        )
        .code("app"));
    }
    attach::present(root).map_err(|m| Outcome::new(Kind::Error, m))?;
    Ok(starts)
}

/// Everything that must be true before money is spent.
pub(super) fn plan(
    c: &mut Ctx,
    root: &Path,
    words: &str,
    hook: bool,
    base: client::Base,
    http: Duration,
) -> Result<Plan, Outcome> {
    let fail = |m: String| Outcome::new(Kind::Error, m);
    let synthetic = repo::is_synthetic(root);
    let model = model_name().map_err(fail)?;
    c.model.clone_from(&model);
    if model.ends_with("-contributor") && !synthetic {
        return Err(fail(format!(
            "the model `{model}` may train on what it is sent, so it is refused unless the folder holds a `.synthetic` marker"
        )));
    }
    let cfg = client::load_config().map_err(fail)?;
    let cap = client::day_cap(&cfg).map_err(fail)?;
    // first: where Torpor is not saving, nothing about the folder (no start script) is news for the user
    if hook {
        let turn = attach::hook_turn(root).map_err(|why| Outcome::new(Kind::Skipped, why))?;
        if attach::promised_at(root, turn.n) {
            return Err(Outcome::new(
                Kind::Skipped,
                "a check from your words at this turn is already saved (or left waiting for its proof by a stopped run)",
            ));
        }
    }
    let starts = local_checks(root)?;
    let key = client::load_key(&cfg).map_err(|m| fail(m).code("key"))?;
    c.key = Some(key.clone());
    Ok(Plan {
        root: root.to_path_buf(),
        ledger_dir: state::state_dir().map_err(fail)?,
        model,
        key,
        base,
        cap,
        starts,
        excerpt: repo::excerpt(root, synthetic),
        words: repo::redact(&cap_words(words), synthetic),
        http,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_long_prompt_keeps_its_start_and_its_end() {
        assert_eq!(cap_words("  login works  "), "login works");
        let long = format!("{}{}{}", "a".repeat(1200), "m".repeat(900), "z".repeat(600));
        let c = cap_words(&long);
        assert!(c.chars().count() <= WORDS_MAX + 8);
        assert!(c.starts_with("aaa") && c.ends_with("zzz") && c.contains("[...]"));
        let accented = "é".repeat(3000);
        assert!(
            cap_words(&accented).chars().count() <= WORDS_MAX + 8,
            "cuts on characters, not bytes"
        );
    }
}
