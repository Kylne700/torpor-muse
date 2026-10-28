//! `say <clip.wav>`: a spoken claim goes to Meta's Muse Voice Transcribe API, and the text it hears goes to the
//! check-writer exactly as a typed claim would (`suggest`). The audio file is sent to Meta; Torpor stores no audio.
//!
//! Exit codes: 2 wrong use; 1 the clip could not be turned into text (bad file, no key, the day cap, an API
//! error), nothing is saved; 3 nothing was heard; otherwise those of `suggest::run`. The call is metered in the
//! same ledger and day cap as the writer.
mod keywords;
#[cfg(test)]
mod tests;
mod upload;
mod wav;

use super::repo;
use keywords::keywords;
use upload::transcribe;

/// Whether the project's words go along when neither flag is given. Set by the measurement in docs/voice.md: the rule
/// (more hits, no more artifacts) held, 11 of 18 clips with the words against 6 without, 2 artifacts against 6.
const KEYWORDS_DEFAULT: bool = true;
const KEYWORDS_MAX: usize = 24;
const PRIVACY: &str =
    "the audio file is sent to Meta's Voice Transcribe API; Torpor stores no audio.";
const FFMPEG: &str = "ffmpeg -i input -ac 1 -ar 24000 -c:a pcm_s16le -map_metadata -1 output.wav";

fn help() -> String {
    format!(
        "usage: torpor-muse say <clip.wav> [--keywords | --no-keywords]\n\n\
         Say what works, get a check. The clip is transcribed by Meta's Muse Voice Transcribe API, what it heard is\n\
         printed as heard: \"...\", and that text goes to `torpor-muse suggest` exactly as if you had typed it\n\
         (same exit codes).\n\n\
         Privacy: {PRIVACY}\n\
         Keywords ({}): up to {KEYWORDS_MAX} words from the names of your code files and the URL paths in your code\n\
         are sent along with the audio, so that the transcription favours your own words. --no-keywords sends\n\
         none, --keywords sends them.\n\n\
         The clip: a WAV file (a path; `-` for standard input is not supported), PCM, mono, 16-bit, 16000 or 24000 Hz,\n\
         at most 10 minutes and 32 MB. To convert one:\n\
         \x20 {FFMPEG}\n\
         Cost: USD 0.18 per audio hour, counted in the same ledger and day cap as `suggest`; the same API key.",
        if KEYWORDS_DEFAULT {
            "on by default"
        } else {
            "off by default"
        }
    )
}

fn bail(msg: &str) -> u8 {
    eprintln!("torpor-muse say: {msg}");
    1
}

pub fn say(args: &[&str]) -> u8 {
    let (mut file, mut kw) = (None, KEYWORDS_DEFAULT);
    for a in args {
        match *a {
            "--help" | "-h" => {
                println!("{}", help());
                return 0;
            }
            "--keywords" => kw = true,
            "--no-keywords" => kw = false,
            f if !f.is_empty() && !f.starts_with("--") && file.is_none() => file = Some(f),
            _ => {
                eprintln!(
                    "usage: torpor-muse say <clip.wav> [--keywords | --no-keywords]  (--help for more)"
                );
                return 2;
            }
        }
    }
    let Some(file) = file else {
        eprintln!(
            "usage: torpor-muse say <clip.wav> [--keywords | --no-keywords]  (--help for more)"
        );
        return 2;
    };
    // a folder the writer would refuse costs nothing
    let root = match repo::resolve_root(None) {
        Ok(r) => r,
        Err(e) => return bail(&e),
    };
    if let Err(o) = super::plan::local_checks(&root) {
        if o.kind == super::types::Kind::Error {
            return bail(&o.message);
        }
        println!("{}: {}", o.kind.word(), o.message);
        return 3;
    }
    let words = if kw { keywords(&root) } else { Vec::new() };
    let text = match transcribe(file, &words) {
        Ok((t, _ms)) => t,
        Err(e) => return bail(&e),
    };
    println!("heard: \"{text}\"");
    if !text.chars().any(char::is_alphanumeric) {
        println!("advisory: nothing was heard in the clip, so no check was made");
        return 3;
    }
    super::run(&text)
}
