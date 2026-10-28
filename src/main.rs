//! torpor-muse: Muse Code hooks that drive the released Torpor engine (a pinned dependency, see engine.lock),
//! plus `suggest`, which asks Muse Spark to write a check from the user's own words, and `say`, which takes them
//! from a spoken clip.

mod doctor;
mod engine;
mod hook;
mod install;
mod ledger;
mod liveness;
mod schema;
mod suggest;
mod transcript;
mod util;

use std::process::ExitCode;

const USAGE: &str = "usage: torpor-muse <command>
  init                    connect Torpor to Muse Code (run once)
  doctor                  check that the connection works
  suggest \"<what works>\"  write a check from your words
  say <clip.wav>          the same, from a spoken clip
  uninstall               disconnect Torpor from Muse Code
  version                 print the version
  hook <Event>            internal: Muse Code runs this, not you";

fn main() -> ExitCode {
    // not `args()`: it panics on an argument that is not UTF-8
    let args: Vec<String> = std::env::args_os()
        .skip(1)
        .map(|a| a.to_string_lossy().into_owned())
        .collect();
    let code = match args
        .iter()
        .map(String::as_str)
        .collect::<Vec<_>>()
        .as_slice()
    {
        ["init"] => install::init(),
        ["uninstall"] => install::uninstall(),
        ["hook", event] => hook::run(event),
        ["doctor"] => doctor::run(),
        // asked for on purpose: stdout, exit 0 (`suggest --help` must not be taken as the words to check)
        ["help" | "--help" | "-h"] | ["suggest", "--help" | "-h"] => {
            println!("{USAGE}");
            0
        }
        ["suggest", words @ ..] if !words.is_empty() => suggest::run(&words.join(" ")),
        ["say", rest @ ..] => suggest::say(rest),
        ["version" | "--version"] => {
            println!("torpor-muse {}", env!("CARGO_PKG_VERSION"));
            0
        }
        _ => {
            eprintln!("{USAGE}");
            2
        }
    };
    ExitCode::from(code)
}
