//! Stops an engine call that ran out of time, and the apps it started.
//!
//! The pinned engine starts the app under test (a server, with its port and memory) in a process group of its own, in the
//! engine's session, and stops it only when the engine exits by itself. Killing the engine's group at a deadline therefore
//! leaves that app running, reparented to init. `kill` stops the engine's group and the group of every descendant of the
//! engine that is still in the engine's session. Left alone: a descendant that started a session of its own (the engine's
//! `gc` does, on purpose, to outlive a hook), a process that is no descendant, and our own group.
//!
//! The engine's group is stopped (SIGSTOP) before the descendants are listed, so nothing in it can start another group
//! meanwhile: once the engine is dead its children are reparented and cannot be found any more.
use std::collections::BTreeSet;
use std::io::Read;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

/// All of `kill`, listing included. A hook has at least 2 s between its engine limit and Muse's own (`hook/events.rs`), and
/// `engine::PIPE_GRACE` comes after; a group left stopped for good would hold the engine's run lock.
const BUDGET: Duration = Duration::from_millis(800);
/// A group that a stopped group's members started just before they stopped is found by the next round.
const ROUNDS: usize = 4;

/// One line of the process table: pid, parent pid, process group.
type Row = (i32, i32, i32);

/// The one place that signals a group. Never 0 or 1 (`kill(-1, _)` reaches every process we may signal) and never our own.
fn signal(pgid: i32, sig: i32) {
    // SAFETY: getpgrp(2) only reads.
    if pgid > 1 && pgid != unsafe { libc::getpgrp() } {
        // SAFETY: kill(2) on a group of the engine's tree. Its members are stopped until this kill, so none can be gone
        // and its id taken by another group.
        unsafe { libc::kill(-pgid, sig) };
    }
}

/// The groups stopped so far. Dropping it kills them all, whichever way `kill_with` ends (a panic included), so that none
/// is left stopped for good.
#[derive(Default)]
struct Frozen(BTreeSet<i32>);

impl Frozen {
    /// Stops group `pgid` unless it is stopped already; true when it was new.
    fn stop(&mut self, pgid: i32) -> bool {
        let new = self.0.insert(pgid);
        if new {
            signal(pgid, libc::SIGSTOP);
        }
        new
    }
}

impl Drop for Frozen {
    fn drop(&mut self) {
        for &g in &self.0 {
            signal(g, libc::SIGKILL);
        }
    }
}

/// The process table, or None when `ps` fails or is not done by `deadline`. `ps -A -o ...=` is the same on Linux and macOS.
fn list(deadline: Instant) -> Option<Vec<Row>> {
    if Instant::now() >= deadline {
        return None;
    }
    let mut ps = Command::new("ps")
        .args(["-A", "-o", "pid=", "-o", "ppid=", "-o", "pgid="])
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .ok()?;
    let mut pipe = ps.stdout.take()?;
    let reader = std::thread::spawn(move || {
        let mut text = String::new();
        let _ = pipe.read_to_string(&mut text);
        text
    });
    let done = loop {
        match ps.try_wait() {
            Ok(Some(status)) => break status.success(),
            Ok(None) if Instant::now() < deadline => std::thread::sleep(Duration::from_millis(2)),
            _ => {
                let _ = ps.kill(); // a `ps` that hangs is ours to stop, by its own pid
                let _ = ps.wait();
                break false;
            }
        }
    };
    let text = reader.join().ok()?;
    let rows: Vec<Row> = text
        .lines()
        .filter_map(|l| {
            let mut n = l.split_whitespace().map(str::parse::<i32>);
            Some((n.next()?.ok()?, n.next()?.ok()?, n.next()?.ok()?))
        })
        .collect();
    (done && !rows.is_empty()).then_some(rows)
}

/// The groups to stop. `root` is the engine's pid, which is also its group. Besides that one: the group of each descendant
/// of the engine that is in its session. Not: our own group `own`, the session's own group, a group whose leader is alive
/// and no descendant (someone else's), ids 0 and 1, and anything whose session cannot be read.
fn groups(rows: &[Row], root: i32, own: i32, session_of: impl Fn(i32) -> i32) -> BTreeSet<i32> {
    let sid = session_of(root);
    let mut kin = BTreeSet::from([root]);
    let mut todo = vec![root];
    while let Some(parent) = todo.pop() {
        for &(pid, ppid, _) in rows {
            if ppid == parent && kin.insert(pid) {
                todo.push(pid);
            }
        }
    }
    let alive: BTreeSet<i32> = rows.iter().map(|r| r.0).collect();
    let mut out = BTreeSet::from([root]);
    for &(pid, _, pg) in rows {
        let someone_elses = alive.contains(&pg) && !kin.contains(&pg);
        if kin.contains(&pid)
            && sid > 0
            && session_of(pid) == sid
            && pg > 1
            && pg != own
            && pg != sid
            && !someone_elses
        {
            out.insert(pg);
        }
    }
    out
}

/// Kills the engine whose pid and group is `pgid`, and what it started (see the top). None = all of it was found and
/// killed. Some = the reason that only the groups found so far, at least the engine's own, were killed. Never panics.
pub fn kill(pgid: i32) -> Option<String> {
    kill_with(pgid, list)
}

fn kill_with(root: i32, list: impl Fn(Instant) -> Option<Vec<Row>>) -> Option<String> {
    // SAFETY: getpgrp(2) only reads.
    let own = unsafe { libc::getpgrp() };
    if root <= 1 || root == own {
        return None;
    }
    // SAFETY: getsid(2) only reads; it returns -1 for a process it cannot read.
    let session_of = |pid: i32| unsafe { libc::getsid(pid) };
    let deadline = Instant::now() + BUDGET;
    let mut frozen = Frozen::default();
    frozen.stop(root);
    if session_of(root) <= 0 {
        return Some(
            "could not read its session, so apps it started in other groups may still run".into(),
        );
    }
    for _ in 0..ROUNDS {
        let Some(rows) = list(deadline) else {
            return Some(
                "could not list the processes, so apps it started in other groups may still run"
                    .into(),
            );
        };
        let mut grew = false;
        for g in groups(&rows, root, own, session_of) {
            grew |= frozen.stop(g);
        }
        if !grew {
            return None;
        }
    }
    Some("it kept starting new groups, so some apps it started may still run".into())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::process::{CommandExt, ExitStatusExt};

    #[test]
    fn the_groups_are_the_engines_and_its_descendants_in_its_session() {
        // (pid, ppid, pgid); the adapter's own group is 50 and the session's id is 200
        let rows = [
            (100, 1, 100),   // the engine
            (101, 100, 101), // the app under test: a group of its own
            (102, 101, 101), // npm, in it
            (103, 102, 103), // node, in a group of its own again
            (104, 100, 104), // `gc`: a session of its own
            (105, 1, 105),   // not a descendant
            (106, 100, 50),  // in the adapter's own group
            (107, 100, 109), // in a group that someone else leads
            (109, 1, 109),
            (110, 100, 200), // in the session's own group
            (111, 100, 111), // its session cannot be read
            (112, 100, 1),   // group 1
        ];
        let session = |pid| match pid {
            104 => 104,
            111 => -1,
            _ => 200,
        };
        assert_eq!(
            groups(&rows, 100, 50, session),
            BTreeSet::from([100, 101, 103])
        );
        // without the engine's own session there is nothing to compare with: its group only
        assert_eq!(groups(&rows, 100, 50, |_| -1), BTreeSet::from([100]));
    }

    #[test]
    fn the_process_table_has_this_process_with_its_parent_and_group() {
        let rows = list(Instant::now() + Duration::from_secs(5)).expect("ps lists the processes");
        let me = i32::try_from(std::process::id()).unwrap();
        let mine = rows.iter().find(|r| r.0 == me).expect("this process");
        // SAFETY: both only read.
        let (ppid, pgid) = unsafe { (libc::getppid(), libc::getpgrp()) };
        assert_eq!((mine.1, mine.2), (ppid, pgid));
        assert!(list(Instant::now()).is_none(), "no time left, no listing");
    }

    #[test]
    fn when_the_processes_cannot_be_listed_the_engines_group_dies_and_the_caller_is_told() {
        struct Group(std::process::Child);
        impl Drop for Group {
            fn drop(&mut self) {
                // the group this test made, by the id of its leader, which is not reaped yet
                signal(i32::try_from(self.0.id()).unwrap(), libc::SIGKILL);
                let _ = self.0.wait();
            }
        }
        let mut engine = Group(
            Command::new("sh")
                .args(["-c", "sleep 30 & wait"])
                .process_group(0)
                .spawn()
                .unwrap(),
        );
        let note = kill_with(i32::try_from(engine.0.id()).unwrap(), |_| None);
        assert!(note.is_some_and(|n| n.contains("could not list")));
        let mut status = None;
        for _ in 0..100 {
            status = engine.0.try_wait().unwrap();
            if status.is_some() {
                break;
            }
            std::thread::sleep(Duration::from_millis(20));
        }
        assert_eq!(status.and_then(|s| s.signal()), Some(libc::SIGKILL));
    }
}
