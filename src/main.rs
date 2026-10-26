//! torpor-muse: Muse Code hooks that drive the released Torpor engine (a pinned dependency, see engine.lock),
//! plus `suggest`, which asks Muse Spark to write a check from the user's own words.

mod compact;
mod doctor;
mod engine;
mod hook;
mod install;
mod ledger;
mod liveness;
mod schema;
mod suggest;

use std::process::ExitCode;

const USAGE: &str = "usage: torpor-muse init | uninstall | hook <Event> | doctor | suggest \"<what works>\" | version";

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
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
        ["suggest", words @ ..] if !words.is_empty() => suggest::run(&words.join(" ")),
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
