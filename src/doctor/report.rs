//! The report: levels and lines, and the small formatters the checks use for time, text and ids.

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Level {
    Ok,
    Info,
    Warn,
    Fix,
}

#[derive(Default, Debug)]
pub struct Report(pub Vec<(Level, String)>);

impl Report {
    pub(crate) fn add(&mut self, l: Level, what: impl Into<String>) {
        self.0.push((l, what.into()));
    }
    /// A problem and the way out, as two lines.
    pub(crate) fn problem(&mut self, l: Level, what: impl AsRef<str>, how: impl AsRef<str>) {
        let (w, h) = (what.as_ref(), how.as_ref());
        self.0.push((l, format!("{w}\n      do: {h}")));
    }
    pub fn fixes(&self) -> usize {
        self.0.iter().filter(|(l, _)| *l == Level::Fix).count()
    }
    pub fn text(&self) -> String {
        let tag = |l: &Level| match l {
            Level::Ok => "ok  ",
            Level::Info => "info",
            Level::Warn => "warn",
            Level::Fix => "FIX ",
        };
        self.0
            .iter()
            .map(|(l, s)| format!("{} {s}\n", tag(l)))
            .collect()
    }
}

pub(crate) fn ago(now_ms: u64, t_ms: u64) -> String {
    let s = now_ms.saturating_sub(t_ms) / 1000;
    match s {
        0..=119 => format!("{s}s"),
        120..=7199 => format!("{}m", s / 60),
        7200..=172_799 => format!("{}h", s / 3600),
        _ => format!("{}d", s / 86_400),
    }
}

/// One line, no control characters, at most `n` characters.
pub(crate) fn clip(s: &str, n: usize) -> String {
    let one: String = s
        .chars()
        .map(|c| if c.is_control() { ' ' } else { c })
        .collect();
    let one = one.trim();
    if one.chars().count() > n {
        format!("{}...", one.chars().take(n).collect::<String>())
    } else {
        one.to_string()
    }
}

pub(crate) fn short(id: &str) -> String {
    id.chars().take(13).collect()
}
