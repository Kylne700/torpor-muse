//! Starting a run from the hook: the detached spawn, the reasons it may not start, and whether the writer is off.

use super::plan::cap_words;
use super::{client, repo, state};
use std::os::unix::process::CommandExt;
use std::path::Path;
use std::process::{Child, Command, Stdio};

/// hook.rs calls this on UserPromptSubmit (main session) after the engine call; returns at once (detached run).
/// Some(kind) = no run started and the user should hear it at the next prompt (`Skip::news`).
pub fn trigger(session_id: &str, cwd: &str, prompt: &str) -> Option<&'static str> {
    let exe = std::env::current_exe().ok()?;
    trigger_with(&exe, session_id, cwd, prompt).err()?.news()
}

/// Why the trigger did not start a run (it never says more than that, and never fails the hook).
#[derive(Debug, PartialEq)]
struct Skip(&'static str);

const BUSY: &str = "a run is already under way here";

const NO_STATE: &str = "no state folder";

const NO_START: &str = "cannot start the run";

impl Skip {
    /// The skips the user is told about: `busy` (the run of an earlier prompt holds the folder) and `start` (a run
    /// could not be started). A short prompt has nothing to check; no project folder, an unusable config or no key
    /// are the "check-writer is off" note.
    fn news(&self) -> Option<&'static str> {
        match self.0 {
            BUSY => Some("busy"),
            NO_STATE | NO_START => Some("start"),
            _ => None,
        }
    }
}

/// Why a prompt in Muse Code would not start the check-writer for want of a key (None = it would, or the cause is
/// another). `env`: a `META_API_KEY` in this process's environment counts, as for the hook; `doctor` runs in a shell,
/// and Muse's hooks may not get that variable, so it passes false.
pub fn writer_off(env: bool) -> Option<String> {
    let cfg = match client::load_config() {
        Ok(c) => c,
        Err(e) => return Some(e),
    };
    let gap = client::key_file_gap(&cfg)?;
    (!(env && client::env_key_ok())).then_some(gap)
}

/// A prompt with less than three characters besides spaces has nothing to check.
fn too_short(prompt: &str) -> bool {
    prompt.chars().filter(|c| !c.is_whitespace()).count() < 3
}

/// Would a run started by this prompt in `cwd` send the prompt to Meta? The local reasons it would not, read the way the
/// trigger and the plan read them: the writer is off (no usable key), the prompt is too short, no project folder, a day
/// cap of 0 (or one that is not a number), no script to start the app with. Not asked here: whether Torpor saves in the
/// folder (the engine's word; the run stops there after `torpor off`, before it sends anything), a run already under
/// way, and a day cap that is used up.
pub fn would_send(cwd: &str, prompt: &str) -> bool {
    !too_short(prompt)
        && writer_off(true).is_none()
        && client::load_config()
            .and_then(|c| client::day_cap(&c))
            .is_ok_and(|cap| cap > 0.0)
        && repo::resolve_root(Some(cwd)).is_ok_and(|root| !repo::start_choices(&root).is_empty())
}

/// Cheap, local checks only (no network, no engine, no model): a prompt with something in it, a usable folder, a key
/// source, no run under way here. Then a detached `suggest -` in its own process group with the prompt in its
/// environment (not on its command line, where every user could read it). The key stays out of its environment when
/// the config names a key file it can read; with no usable key file, a `META_API_KEY` of ours goes along.
/// The judgment whether the prompt says "this works" is the model's, in the run.
fn trigger_with(exe: &Path, session_id: &str, cwd: &str, prompt: &str) -> Result<Child, Skip> {
    if too_short(prompt) {
        return Err(Skip("the prompt is too short"));
    }
    let root = repo::resolve_root(Some(cwd)).map_err(|_| Skip("no usable project folder"))?;
    let cfg = client::load_config().map_err(|_| Skip("the config file is not usable"))?;
    let file_ok = client::key_file_gap(&cfg).is_none();
    if !file_ok && !client::env_key_ok() {
        return Err(Skip("no key file in the config, and no META_API_KEY"));
    }
    let ws = state::ws_dir(&root).map_err(|_| Skip(NO_STATE))?;
    match state::try_lock(&ws) {
        Ok(Some(_free)) => {}
        _ => return Err(Skip(BUSY)),
    }
    let mut cmd = Command::new(exe);
    cmd.args(["suggest", "-"])
        .current_dir(&root)
        .env_remove("CLAUDECODE")
        .env("TORPOR_MUSE_FROM_HOOK", session_id)
        .env("TORPOR_MUSE_WORDS", cap_words(prompt))
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .process_group(0);
    if file_ok {
        cmd.env_remove("META_API_KEY");
    }
    crate::util::spawn_retry(&mut cmd).map_err(|_| Skip(NO_START))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::os::unix::fs::PermissionsExt;
    use std::path::PathBuf;
    use std::time::{Duration, Instant};

    fn tmp(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("tm-trigger-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&d);
        fs::create_dir_all(&d).unwrap();
        d
    }

    #[test]
    fn the_trigger_returns_fast_and_starts_a_detached_run_without_the_key_or_the_prompt_on_the_command_line()
     {
        let d = tmp("spawn");
        let (home, proj, state) = (d.join("home"), d.join("proj"), d.join("state"));
        for x in [&home, &proj] {
            fs::create_dir_all(x).unwrap();
        }
        let keyfile = d.join("key");
        fs::write(&keyfile, "LLM_notarealkey000\n").unwrap();
        fs::set_permissions(&keyfile, fs::Permissions::from_mode(0o600)).unwrap();
        fs::create_dir_all(home.join(".config/torpor-muse")).unwrap();
        let cfg = home.join(".config/torpor-muse/config");
        fs::write(&cfg, format!("key_file = {}\n", keyfile.display())).unwrap();
        fs::set_permissions(&cfg, fs::Permissions::from_mode(0o600)).unwrap();
        let fake = d.join("fake-muse");
        fs::write(
            &fake,
            "#!/bin/sh\nsleep 0.3\necho \"$@\" > seen-args\nenv > seen-env\npwd > seen-pwd\n",
        )
        .unwrap();
        fs::set_permissions(&fake, fs::Permissions::from_mode(0o755)).unwrap();

        let _g = state::ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        // SAFETY: every test that touches the environment holds ENV_LOCK.
        unsafe {
            std::env::set_var("HOME", &home);
            std::env::set_var("TORPOR_MUSE_STATE", &state);
            std::env::set_var("META_API_KEY", "LLM_envkeyshouldnotpass");
            std::env::remove_var("CLAUDE_PROJECT_DIR");
        }
        let prompt = "the login works now, thanks";
        let cwd = proj.to_string_lossy().into_owned();
        let mut times = Vec::new();
        let mut children = Vec::new();
        for i in 0..5 {
            let t = Instant::now();
            let r = trigger_with(&fake, "sess-1", &cwd, prompt);
            times.push(t.elapsed());
            children.push(r.unwrap_or_else(|s| panic!("trigger {i} did not start a run: {s:?}")));
        }
        times.sort();
        assert!(
            times[2] < Duration::from_millis(50),
            "median {:?}",
            times[2]
        );
        for mut ch in children {
            ch.wait().unwrap();
        }
        let seen = |n: &str| fs::read_to_string(proj.join(n)).unwrap_or_default();
        assert_eq!(
            seen("seen-args").trim(),
            "suggest -",
            "the prompt is not on the command line"
        );
        let env_seen = seen("seen-env");
        assert!(env_seen.contains("TORPOR_MUSE_FROM_HOOK=sess-1"));
        assert!(env_seen.contains(&format!("TORPOR_MUSE_WORDS={prompt}")));
        assert!(!env_seen.contains("META_API_KEY"), "the child gets no key");
        assert!(seen("seen-pwd").contains("proj"));

        // too short, no key file, a run under way: no spawn
        assert_eq!(
            trigger_with(&fake, "s", &cwd, "ok").unwrap_err(),
            Skip("the prompt is too short")
        );
        let ws = state::ws_dir(&repo::resolve_root(Some(&cwd)).unwrap()).unwrap();
        let held = state::try_lock(&ws).unwrap().unwrap();
        assert_eq!(
            trigger_with(&fake, "s", &cwd, prompt).unwrap_err(),
            Skip("a run is already under way here")
        );
        drop(held);
        fs::set_permissions(&keyfile, fs::Permissions::from_mode(0o644)).unwrap();
        unsafe { std::env::remove_var("META_API_KEY") };
        assert_eq!(
            trigger_with(&fake, "s", &cwd, prompt).unwrap_err(),
            Skip("no key file in the config, and no META_API_KEY")
        );

        // no usable key file, but a key in our environment: the run gets it (the by-hand route, break4 MAJOR-2)
        fs::remove_file(proj.join("seen-env")).unwrap();
        unsafe { std::env::set_var("META_API_KEY", "LLM_envkeyfallback") };
        trigger_with(&fake, "s", &cwd, prompt)
            .unwrap()
            .wait()
            .unwrap();
        assert!(seen("seen-env").contains("META_API_KEY=LLM_envkeyfallback"));
        unsafe {
            std::env::remove_var("META_API_KEY");
            std::env::remove_var("TORPOR_MUSE_STATE");
        }
        let _ = fs::remove_dir_all(&d);
    }

    #[test]
    fn only_a_busy_folder_or_a_run_that_would_not_start_is_news_for_the_user() {
        assert_eq!(Skip(BUSY).news(), Some("busy"));
        assert_eq!(Skip(NO_STATE).news(), Some("start"));
        assert_eq!(Skip(NO_START).news(), Some("start"));
        for quiet in [
            "the prompt is too short",
            "no usable project folder",
            "the config file is not usable",
            "no key file in the config, and no META_API_KEY",
        ] {
            assert_eq!(Skip(quiet).news(), None, "{quiet}");
        }
    }

    #[test]
    fn the_writer_is_off_says_why_unless_a_key_file_or_the_environment_has_a_key() {
        let d = tmp("off");
        let home = d.join("home");
        fs::create_dir_all(home.join(".config/torpor-muse")).unwrap();
        let (cfg, key) = (home.join(".config/torpor-muse/config"), d.join("key"));
        fs::write(&key, "LLM_notarealkey000\n").unwrap();
        let mode = |p: &Path, m| fs::set_permissions(p, fs::Permissions::from_mode(m)).unwrap();
        let config = |text: &str| {
            fs::write(&cfg, text).unwrap();
            mode(&cfg, 0o600);
        };
        let _g = state::ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        // SAFETY: every test that touches the environment holds ENV_LOCK.
        unsafe {
            std::env::set_var("HOME", &home);
            std::env::remove_var("META_API_KEY");
        }
        let off = |env| writer_off(env).unwrap_or_default();
        assert!(off(true).contains("no `key_file"), "no config");
        config("key_file = ~/nokey\n");
        assert!(
            off(true).contains("cannot read"),
            "a key file that is not there: {}",
            off(true)
        );
        config(&format!("key_file = {}\n", key.display()));
        mode(&key, 0o644);
        assert!(off(true).contains("only you can read"));
        mode(&key, 0o600);
        assert_eq!(writer_off(true), None);
        assert_eq!(writer_off(false), None);
        config("key_file = ~/../key\n");
        assert_eq!(writer_off(false), None, "`~/` is the home folder");
        config("keyfile = x\n");
        assert!(off(true).contains("unknown or empty setting"));
        config("# nothing\n");
        unsafe { std::env::set_var("META_API_KEY", "LLM_envkeyfallback") };
        assert_eq!(
            writer_off(true),
            None,
            "a key in the environment counts for the hook"
        );
        assert!(
            writer_off(false).is_some(),
            "but not for doctor, which runs in a shell"
        );
        unsafe { std::env::remove_var("META_API_KEY") };
        let _ = fs::remove_dir_all(&d);
    }
}
