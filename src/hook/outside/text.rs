//! Finds `rm` aimed at the home folder or `/` in text an AI writes to a file (incident #105342 in research/71c: a test
//! holding `"20; rm -rf ~"`). A plain text scan, so quotes and string literals do not hide it.

const HOME: &[&str] = &[
    "~",
    "~/",
    "~/*",
    "$HOME",
    "$HOME/",
    "$HOME/*",
    "${HOME}",
    "${HOME}/",
    "${HOME}/*",
];
const ROOT: &[&str] = &["/", "/*"];
/// Words in front of a command that only run it.
const LEAD: &[&str] = &["sudo", "exec", "command", "{", "then", "do", "else"];

/// The first such command, as `rm -rf ~`, and whether it aims at the home folder (else at `/`).
pub(super) fn find(text: &str) -> Option<(String, bool)> {
    let plain: String = text
        .chars()
        .map(|c| {
            if matches!(c, '"' | '\'' | '\\') {
                ' '
            } else {
                c
            }
        })
        .collect();
    for part in plain.split(['\n', ';', '&', '|', '(', ')', '`']) {
        let mut ws = part.split_whitespace().skip_while(|w| LEAD.contains(w));
        if ws.next().map(|v| v.rsplit('/').next().unwrap_or(v)) != Some("rm") {
            continue;
        }
        let (mut cmd, mut opts) = (String::from("rm "), true);
        for w in ws {
            if opts && w.len() > 1 && w.starts_with('-') {
                opts = w != "--";
                cmd.push_str(w);
                cmd.push(' ');
            } else if HOME.contains(&w) || ROOT.contains(&w) {
                cmd.push_str(w);
                return Some((cmd, HOME.contains(&w)));
            }
        }
    }
    None
}
