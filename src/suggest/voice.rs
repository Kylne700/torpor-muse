//! `say <clip.wav>`: a spoken claim goes to Meta's Muse Voice Transcribe API, and the text it hears goes to the
//! check-writer exactly as a typed claim would (`suggest`). The audio file is sent to Meta; Torpor stores no audio.
//!
//! Exit codes: 2 wrong use; 1 the clip could not be turned into text (bad file, no key, the day cap, an API
//! error), nothing is saved; 3 nothing was heard; otherwise those of `suggest::run`. The call is metered in the
//! same ledger and day cap as the writer.
use super::client::{self, Bad, Key, Net, Reading, Reply};
use super::{prompt, repo, state};
use crate::ledger::{ASR_MODEL, Charge, Ledger, asr_cost};
use serde_json::{Value, json};
use std::collections::BTreeMap;
use std::fs;
use std::path::Path;
use std::time::{Duration, Instant};

/// Whether the project's words go along when neither flag is given. Set by the measurement in docs/voice.md.
const KEYWORDS_DEFAULT: bool = false;
const KEYWORDS_MAX: usize = 24;
/// Limits of the API: more bytes get a 413, more audio a 400.
const MAX_FILE: u64 = 32_000_000;
const MAX_MS: u64 = 600_000;
const MAX_REPLY: u64 = 1_000_000;
const PRIVACY: &str =
    "the audio file is sent to Meta's Voice Transcribe API; Torpor stores no audio.";
const FFMPEG: &str = "ffmpeg -i input -ac 1 -ar 24000 -c:a pcm_s16le -map_metadata -1 output.wav";
/// The code files whose names and URL paths give the keywords.
const CODE: [&str; 18] = [
    "js", "mjs", "cjs", "ts", "tsx", "jsx", "py", "rb", "php", "go", "java", "rs", "cs", "vue",
    "html", "ejs", "hbs", "pug",
];

fn help() -> String {
    format!(
        "usage: torpor-muse say <clip.wav> [--keywords | --no-keywords]\n\n\
         Say what works, get a check. The clip is transcribed by Meta's Muse Voice Transcribe API, what it heard is\n\
         printed as heard: \"...\", and that text goes to `torpor-muse suggest` exactly as if you had typed it\n\
         (same exit codes).\n\n\
         Privacy: {PRIVACY}\n\
         With --keywords, up to {KEYWORDS_MAX} words from the names of your code files and the URL paths in your code\n\
         are sent along with the audio, so that the transcription favours your own words. --no-keywords sends\n\
         none. Without either flag: {}.\n\n\
         The clip: a WAV file, PCM, mono, 16-bit, 16000 or 24000 Hz, at most 10 minutes and 32 MB. To convert one:\n\
         \x20 {FFMPEG}\n\
         Cost: USD 0.18 per audio hour, counted in the same ledger and day cap as `suggest`; the same API key.",
        if KEYWORDS_DEFAULT {
            "keywords are sent"
        } else {
            "no keywords are sent"
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
            f if !f.starts_with("--") && file.is_none() => file = Some(f),
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

// ---------------------------------------------------------------- the clip

/// The length in ms of a WAV the API takes, or why it will not take it: PCM, mono, 16-bit, 16000 or 24000 Hz, at
/// most 10 minutes. Extra chunks (LIST and so on) are skipped; a data chunk that claims more than the file holds
/// counts as what is there.
fn check_wav(b: &[u8]) -> Result<u64, String> {
    if b.len() < 12 || &b[..4] != b"RIFF" || &b[8..12] != b"WAVE" {
        return Err("this is not a WAV file".into());
    }
    let u16_at = |at: usize| u16::from_le_bytes([b[at], b[at + 1]]);
    let u32_at = |at: usize| u32::from_le_bytes([b[at], b[at + 1], b[at + 2], b[at + 3]]);
    let (mut fmt, mut at) = (None, 12);
    while at + 8 <= b.len() {
        let size = u32_at(at + 4) as usize;
        let body = at + 8;
        match &b[at..at + 4] {
            b"fmt " if size >= 16 && body + 16 <= b.len() => {
                fmt = Some((
                    u16_at(body),
                    u16_at(body + 2),
                    u32_at(body + 4),
                    u16_at(body + 14),
                ));
            }
            b"data" => {
                let Some((tag, channels, rate, bits)) = fmt else {
                    return Err("this WAV file has its audio before its format".into());
                };
                if tag != 1 {
                    return Err(format!(
                        "only plain PCM WAV is taken (this file uses format {tag})"
                    ));
                }
                if channels != 1 {
                    return Err(format!(
                        "the audio must be mono (this has {channels} channels)"
                    ));
                }
                if bits != 16 {
                    return Err(format!("the audio must be 16-bit (this is {bits}-bit)"));
                }
                if rate != 16_000 && rate != 24_000 {
                    return Err(format!(
                        "the sample rate must be 16000 or 24000 Hz (this is {rate} Hz)"
                    ));
                }
                let bytes = size.min(b.len() - body) as u64;
                if bytes == 0 {
                    return Err("this WAV file holds no audio".into());
                }
                let ms = bytes * 1000 / (u64::from(rate) * 2);
                if ms > MAX_MS {
                    return Err(format!(
                        "the clip is {} minutes long; the limit is 10",
                        ms / 60_000
                    ));
                }
                return Ok(ms);
            }
            _ => {}
        }
        at = body + size + (size & 1); // chunks are padded to an even size
    }
    Err("this WAV file has no audio data".into())
}

/// The bytes of the clip and its length in ms, or the one line that says why not (with the ffmpeg command).
fn read_clip(path: &str) -> Result<(Vec<u8>, u64), String> {
    let hint = |why: String| format!("{why}. To convert it: {FFMPEG}");
    let meta = fs::metadata(path).map_err(|e| format!("cannot open {path}: {e}"))?;
    if !meta.is_file() {
        return Err(format!("{path} is not a file"));
    }
    if meta.len() > MAX_FILE {
        return Err(hint(format!(
            "{path} is {} MB; the limit is 32 MB",
            meta.len() / 1_000_000
        )));
    }
    let bytes = fs::read(path).map_err(|e| format!("cannot read {path}: {e}"))?;
    let ms = check_wav(&bytes).map_err(|why| hint(format!("{path}: {why}")))?;
    Ok((bytes, ms))
}

// ---------------------------------------------------------------- the words of the project

/// Lowercase ASCII words of 3 to 15 letters; camelCase and every other character are word breaks.
/// shortcut: ASCII letters only (the plan is English), so a word with an accent is cut at the accent. Upgrade with the languages.
fn words(s: &str) -> Vec<String> {
    let (mut out, mut cur, mut prev_lower) = (Vec::new(), String::new(), false);
    for c in s.chars() {
        if c.is_ascii_alphabetic() {
            if c.is_ascii_uppercase() && prev_lower {
                out.push(std::mem::take(&mut cur));
            }
            prev_lower = c.is_ascii_lowercase();
            cur.push(c.to_ascii_lowercase());
        } else {
            prev_lower = false;
            out.push(std::mem::take(&mut cur));
        }
    }
    out.push(cur);
    out.retain(|w| (3..=15).contains(&w.len()));
    out
}

/// The string literals that start with a slash: a quote, `/`, and on to the same quote (a template literal stops
/// at its first `${`, any literal at the end of the line). These are the URL paths of the code.
fn url_paths(text: &str) -> Vec<&str> {
    let (b, mut out, mut i) = (text.as_bytes(), Vec::new(), 0);
    while i + 1 < b.len() {
        if matches!(b[i], b'\'' | b'"' | b'`') && b[i + 1] == b'/' {
            let rest = &text[i + 1..];
            let end = rest
                .bytes()
                .zip(0..)
                .find(|&(c, j)| {
                    c == b[i] || c == b'\n' || (b[i] == b'`' && rest[j..].starts_with("${"))
                })
                .map_or(rest.len(), |(_, j)| j);
            out.push(repo::cut(&rest[..end], 120));
            i += 1 + end;
        }
        i += 1;
    }
    out
}

/// Up to 24 words of the project's own vocabulary, for the transcription to favour: the words in the names of its
/// code files and in the URL paths written in its code. The most repeated first, then alphabetical, so the same
/// project always gives the same list. Everything read is what the writer reads too (listed, redacted, capped).
/// shortcut: no stop-word list (`server` and `index` count like any word), no care for plurals, and test titles,
/// saved checks and READMEs are not read. Upgrade when a measurement shows it matters.
fn keywords(root: &Path) -> Vec<String> {
    let synthetic = repo::is_synthetic(root);
    let mut count: BTreeMap<String, u32> = BTreeMap::new();
    for rel in repo::allowed_files(root) {
        if !CODE.contains(&rel.rsplit('.').next().unwrap_or("")) {
            continue;
        }
        let stem = rel
            .rsplit('/')
            .next()
            .unwrap_or(&rel)
            .split('.')
            .next()
            .unwrap_or("");
        let text = repo::read_capped(root, &rel, synthetic).unwrap_or_default();
        let text = text.replace(repo::MASK, " ");
        for w in words(stem)
            .into_iter()
            .chain(url_paths(&text).into_iter().flat_map(words))
        {
            *count.entry(w).or_default() += 1;
        }
    }
    let mut ranked: Vec<(String, u32)> = count.into_iter().collect();
    ranked.sort_by_key(|(_, n)| std::cmp::Reverse(*n)); // stable: equal counts stay alphabetical
    ranked
        .into_iter()
        .take(KEYWORDS_MAX)
        .map(|(w, _)| w)
        .collect()
}

// ---------------------------------------------------------------- the call

fn contains(hay: &[u8], needle: &[u8]) -> bool {
    hay.windows(needle.len()).any(|w| w == needle)
}

/// The multipart body (a `request` part and an `audio` part) and its boundary, which is not in the audio. The audio
/// part always has the same file name, never the user's.
fn multipart(wav: &[u8], keywords: &[String]) -> (String, Vec<u8>) {
    let mut request = json!({"model": ASR_MODEL, "audioEncoding": "WAV", "mode": "PUSH_TO_TALK"});
    if !keywords.is_empty() {
        request["keywords"] = json!(keywords);
    }
    let boundary = loop {
        let b = format!("torpor-muse-{}{}", prompt::nonce(), prompt::nonce());
        if !contains(wav, b.as_bytes()) {
            break b;
        }
    };
    let mut body = format!(
        "--{boundary}\r\nContent-Disposition: form-data; name=\"request\"\r\nContent-Type: application/json\r\n\r\n{request}\r\n\
         --{boundary}\r\nContent-Disposition: form-data; name=\"audio\"; filename=\"audio.wav\"\r\nContent-Type: audio/wav\r\n\r\n"
    )
    .into_bytes();
    body.extend_from_slice(wav);
    body.extend_from_slice(format!("\r\n--{boundary}--\r\n").as_bytes());
    (boundary, body)
}

/// The same kind of call as `client::post`: no redirects (the key must not travel on), a 4xx or 5xx is a reply.
fn post_audio(
    base: &client::Base,
    key: &Key,
    boundary: &str,
    body: &[u8],
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
    let mut resp = agent
        .post(format!("{}/v1/asr/transcribe", base.url))
        .header("Authorization", key.bearer())
        .header("Accept", "application/json")
        .header(
            "Content-Type",
            format!("multipart/form-data; boundary={boundary}"),
        )
        .send(body)
        .map_err(client::net)?;
    let status = resp.status().as_u16();
    let retry_after = resp
        .headers()
        .get("retry-after")
        .and_then(|v| v.to_str().ok())
        .map(String::from);
    let body = resp
        .body_mut()
        .with_config()
        .limit(MAX_REPLY)
        .read_to_string()
        .map_err(client::net)?;
    Ok(Reply {
        status,
        retry_after,
        body,
    })
}

/// One answer, read: what it cost, what was heard, and the audio length the server billed (when it said).
/// The error statuses are the writer's, plus 406 and 413.
fn read_reply(r: &Reply, worst: f64) -> (Reading, Option<u64>) {
    let refused = |m: &str| Reading {
        status: format!("http_{}", r.status),
        charge: Charge::Free,
        outcome: Err(Bad::Request(m.into())),
    };
    match r.status {
        406 => {
            return (
                refused("the API does not take the answer format asked for (HTTP 406)"),
                None,
            );
        }
        413 => {
            return (
                refused("the audio is too large for the API (HTTP 413); the limit is 32 MB"),
                None,
            );
        }
        200 => {}
        _ => return (client::interpret(r, worst), None),
    }
    let v: Option<Value> = serde_json::from_str(&r.body).ok();
    let Some(text) = v.as_ref().and_then(|v| v["transcript"].as_str()) else {
        let bad = Bad::Malformed("the Meta API answered without a transcript".into());
        let reading = Reading {
            status: "http_200".into(),
            charge: Charge::Estimate(worst),
            outcome: Err(bad),
        };
        return (reading, None);
    };
    let ms = v
        .as_ref()
        .and_then(|v| v["audioDurationMs"].as_f64())
        .map(|x| x.max(0.0).ceil() as u64);
    let reading = Reading {
        status: "http_200".into(),
        charge: ms.map_or(Charge::Estimate(worst), |ms| Charge::Usd(asr_cost(ms))),
        outcome: Ok(text.to_string()),
    };
    (reading, ms)
}

/// Everything `say` does before the writer: check the clip, the day cap and the key, send, write the ledger row.
/// Returns what was heard and the audio length in ms. `Err` is the one line to show.
fn transcribe(clip: &str, words: &[String]) -> Result<(String, u64), String> {
    let (wav, header_ms) = read_clip(clip)?;
    let base = client::api_base()?;
    let http = if base.loopback {
        super::seconds("TORPOR_MUSE_HTTP_TIMEOUT_S").unwrap_or(super::HTTP_TIMEOUT)
    } else {
        super::HTTP_TIMEOUT
    };
    let cfg = client::load_config()?;
    let cap = client::day_cap(&cfg)?;
    let key = client::load_key(&cfg)?;
    let dir = state::state_dir()?;
    let (boundary, body) = multipart(&wav, words);
    drop(wav);
    let worst = asr_cost(header_ms);
    let mut ledger = Ledger::open(&dir)?;
    ledger.preflight_usd(worst, cap)?;
    let sent = Instant::now();
    let (reading, server_ms) = match post_audio(&base, &key, &boundary, &body, http) {
        Ok(r) => read_reply(&r, worst),
        Err(n) => (client::from_net(n, worst), None),
    };
    let audio_ms = server_ms.unwrap_or(header_ms);
    let kw = if words.is_empty() {
        "off".into()
    } else {
        words.len().to_string()
    };
    let mut note = format!(
        "say: audio {audio_ms} ms, keywords {kw}, {:.1}s",
        sent.elapsed().as_secs_f64()
    );
    if let Err(bad) = &reading.outcome {
        note = format!("{note}: {}", key.scrub(&client::clean(&bad.reason(), 120)));
    }
    ledger.record(ASR_MODEL, &reading.status, reading.charge, &note)?;
    drop(ledger); // the writer takes the same lock
    match reading.outcome {
        Ok(t) => Ok((key.scrub(&client::clean(&t, 1500)), audio_ms)),
        Err(bad) => Err(key.scrub(&bad.reason())),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    /// A WAV with `data` bytes of audio and the given format; `extra` is a chunk put before `data`.
    fn wav_with(
        tag: u16,
        channels: u16,
        rate: u32,
        bits: u16,
        data: usize,
        extra: &[u8],
    ) -> Vec<u8> {
        let mut fmt = Vec::new();
        fmt.extend_from_slice(&tag.to_le_bytes());
        fmt.extend_from_slice(&channels.to_le_bytes());
        fmt.extend_from_slice(&rate.to_le_bytes());
        fmt.extend_from_slice(&(rate * u32::from(channels) * u32::from(bits) / 8).to_le_bytes());
        fmt.extend_from_slice(&(channels * bits / 8).to_le_bytes());
        fmt.extend_from_slice(&bits.to_le_bytes());
        let mut w = b"RIFF\0\0\0\0WAVEfmt ".to_vec();
        w.extend_from_slice(&16u32.to_le_bytes());
        w.extend_from_slice(&fmt);
        w.extend_from_slice(extra);
        w.extend_from_slice(b"data");
        w.extend_from_slice(&u32::try_from(data).unwrap().to_le_bytes());
        w.resize(w.len() + data, 0);
        w
    }

    fn good(rate: u32, ms: usize) -> Vec<u8> {
        wav_with(1, 1, rate, 16, ms * rate as usize * 2 / 1000, &[])
    }

    #[test]
    fn a_good_wav_is_taken_with_its_length() {
        assert_eq!(check_wav(&good(24_000, 2580)), Ok(2580));
        assert_eq!(check_wav(&good(16_000, 500)), Ok(500));
        assert_eq!(
            check_wav(&good(16_000, 600_000)),
            Ok(600_000),
            "10 minutes exactly"
        );
        // a LIST chunk of odd size (padded to even) before the data, as ffmpeg writes one
        let mut list = b"LIST".to_vec();
        list.extend_from_slice(&5u32.to_le_bytes());
        list.extend_from_slice(b"INFOx\0");
        assert_eq!(
            check_wav(&wav_with(1, 1, 24_000, 16, 48_000, &list)),
            Ok(1000)
        );
        // a data chunk that claims more than the file holds (streamed or cut): what is there counts
        let mut cut = good(24_000, 1000);
        cut.truncate(44 + 24_000);
        let len = cut.len();
        cut[len - 24_000 - 4..len - 24_000].copy_from_slice(&u32::MAX.to_le_bytes());
        assert_eq!(check_wav(&cut), Ok(500));
    }

    #[test]
    fn other_wavs_are_refused_with_the_reason() {
        let no = |b: &[u8], part: &str| {
            let e = check_wav(b).unwrap_err();
            assert!(e.contains(part), "{e:?} should mention {part:?}");
        };
        no(&wav_with(1, 2, 24_000, 16, 96_000, &[]), "mono");
        no(&wav_with(1, 1, 24_000, 8, 24_000, &[]), "16-bit");
        no(&wav_with(1, 1, 44_100, 16, 88_200, &[]), "16000 or 24000");
        no(&wav_with(3, 1, 24_000, 16, 48_000, &[]), "PCM");
        no(&wav_with(1, 1, 24_000, 16, 0, &[]), "no audio");
        no(&good(16_000, 600_001), "limit is 10");
        no(b"not a wav at all, just text", "not a WAV");
        no(b"", "not a WAV");
        let mut no_data = good(24_000, 100);
        no_data.truncate(36);
        no(&no_data, "no audio data");
        let mut late = b"RIFF\0\0\0\0WAVEdata".to_vec();
        late.extend_from_slice(&4u32.to_le_bytes());
        late.extend_from_slice(&[0; 4]);
        no(&late, "before its format");
    }

    #[test]
    fn the_file_is_checked_for_size_before_it_is_read_and_a_refusal_names_the_conversion() {
        let d = std::env::temp_dir().join(format!("tm-voice-clip-{}", std::process::id()));
        let _ = fs::remove_dir_all(&d);
        fs::create_dir_all(&d).unwrap();
        let big = d.join("big.wav");
        fs::File::create(&big)
            .unwrap()
            .set_len(MAX_FILE + 1)
            .unwrap(); // sparse: never read
        let e = read_clip(big.to_str().unwrap()).unwrap_err();
        assert!(e.contains("32 MB") && e.contains(FFMPEG), "{e}");
        let text = d.join("a.wav");
        fs::write(&text, "hello").unwrap();
        let e = read_clip(text.to_str().unwrap()).unwrap_err();
        assert!(e.contains("not a WAV") && e.contains(FFMPEG), "{e}");
        assert!(
            read_clip(d.join("missing.wav").to_str().unwrap())
                .unwrap_err()
                .contains("cannot open")
        );
        assert!(
            read_clip(d.to_str().unwrap())
                .unwrap_err()
                .contains("not a file")
        );
        let ok = d.join("ok.wav");
        fs::write(&ok, good(24_000, 1500)).unwrap();
        assert_eq!(read_clip(ok.to_str().unwrap()).unwrap().1, 1500);
        let _ = fs::remove_dir_all(&d);
    }

    /// Splits a multipart body into (name, headers, content) the simple way, to look inside what `multipart` built.
    fn parts(boundary: &str, body: &[u8]) -> Vec<(String, String, Vec<u8>)> {
        let delim = format!("--{boundary}").into_bytes();
        let mut at: Vec<usize> = (0..body.len().saturating_sub(delim.len()))
            .filter(|&i| body[i..].starts_with(&delim))
            .collect();
        assert!(body.ends_with(format!("--{boundary}--\r\n").as_bytes()));
        at.push(body.len());
        at.windows(2)
            .take(at.len() - 2)
            .map(|w| {
                let part = &body[w[0] + delim.len() + 2..w[1] - 2];
                let split = part.windows(4).position(|x| x == b"\r\n\r\n").unwrap();
                let head = String::from_utf8_lossy(&part[..split]).into_owned();
                let name = head
                    .split("name=\"")
                    .nth(1)
                    .unwrap()
                    .split('"')
                    .next()
                    .unwrap()
                    .to_string();
                (name, head, part[split + 4..].to_vec())
            })
            .collect()
    }

    #[test]
    fn the_body_has_a_request_part_and_an_audio_part_and_a_boundary_the_audio_lacks() {
        let mut audio = good(24_000, 100);
        audio.extend_from_slice(b"--torpor-muse-0000"); // looks like the start of a boundary, still not the one picked
        let (boundary, body) = multipart(&audio, &["login".into(), "alice".into()]);
        assert!(!contains(&audio, boundary.as_bytes()));
        let p = parts(&boundary, &body);
        assert_eq!(
            p.iter().map(|x| x.0.as_str()).collect::<Vec<_>>(),
            ["request", "audio"]
        );
        assert!(p[0].1.contains("Content-Type: application/json"));
        let req: Value = serde_json::from_slice(&p[0].2).unwrap();
        assert_eq!(
            req,
            json!({"model": "muse-voice-transcribe-1.0", "audioEncoding": "WAV", "mode": "PUSH_TO_TALK",
                   "keywords": ["login", "alice"]})
        );
        assert!(
            p[1].1.contains("filename=\"audio.wav\"") && p[1].1.contains("Content-Type: audio/wav")
        );
        assert_eq!(p[1].2, audio);
        // no keywords: no `keywords` field at all, and no language setting either way
        let (b2, body2) = multipart(&audio, &[]);
        let req2: Value = serde_json::from_slice(&parts(&b2, &body2)[0].2).unwrap();
        assert_eq!(
            req2,
            json!({"model": "muse-voice-transcribe-1.0", "audioEncoding": "WAV", "mode": "PUSH_TO_TALK"})
        );
        assert_ne!(boundary, b2, "a fresh boundary every time");
    }

    #[test]
    fn words_split_at_camel_case_and_drop_the_very_short_and_the_very_long() {
        assert_eq!(words("userStore"), ["user", "store"]);
        assert_eq!(words("api/Login-v2_me"), ["api", "login"]);
        assert_eq!(words("id to ab"), Vec::<String>::new());
        assert_eq!(
            words("abcdefghijklmnop abcdefghijklmno"),
            ["abcdefghijklmno"]
        );
        assert_eq!(words("HTTPServer"), ["httpserver"]);
    }

    #[test]
    fn url_paths_are_the_literals_that_start_with_a_slash() {
        let code = "app.get('/api/login', f);\napp.post(\"/logout\", g);\nfetch(`/api/notes/${id}/x`);\nconst a = '/';\nlet s = 'x' + '/' + y;\nconst t = 'no/slash/first';\n// it's '/commented' too\n";
        assert_eq!(
            url_paths(code),
            [
                "/api/login",
                "/logout",
                "/api/notes/",
                "/",
                "/",
                "/commented"
            ]
        );
    }

    fn project(name: &str, files: &[(&str, &str)]) -> std::path::PathBuf {
        let d = std::env::temp_dir().join(format!("tm-voice-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&d);
        fs::create_dir_all(&d).unwrap();
        for (f, text) in files {
            fs::File::create(d.join(f))
                .unwrap()
                .write_all(text.as_bytes())
                .unwrap();
        }
        d
    }

    #[test]
    fn keywords_are_the_projects_own_words_the_same_every_time() {
        let d = project(
            "kw",
            &[
                (
                    "server.js",
                    "app.get('/api/login', a);\napp.post('/api/notes', b);\nconst k = '/hook/sk-0123456789abcdef0123456789abcdef';",
                ),
                ("userStore.js", "module.exports = {};"),
                ("README.md", "# Notes\nPOST '/readme/only'"), // not code: not read
                ("data.json", "{\"url\": \"/json/only\"}"),    // not code: not read
                ("notes-secret.js", "app.get('/secret/only')"), // the writer never reads it either
            ],
        );
        let first = keywords(&d);
        assert_eq!(first, ["api", "login", "notes", "server", "store", "user"]);
        assert_eq!(first, keywords(&d), "deterministic");
        // the key and the path it sits in are masked together: nothing of either is sent
        for w in ["redacted", "abcdef", "hook"] {
            assert!(!first.contains(&w.to_string()), "{w} must not be a keyword");
        }
        let _ = fs::remove_dir_all(&d);
    }

    #[test]
    fn keywords_are_capped_and_the_most_repeated_come_first() {
        let names: Vec<String> = (0..40)
            .map(|i| format!("{}{}x", (b'a' + i / 26) as char, (b'a' + i % 26) as char))
            .collect();
        let mut files: Vec<(String, String)> = names
            .iter()
            .map(|n| (format!("{n}.js"), "x".into()))
            .collect();
        files.push(("zzz.js".into(), "f('/popular'); g('/popular');".into()));
        let refs: Vec<(&str, &str)> = files
            .iter()
            .map(|(a, b)| (a.as_str(), b.as_str()))
            .collect();
        let d = project("cap", &refs);
        let k = keywords(&d);
        assert_eq!(k.len(), KEYWORDS_MAX);
        assert_eq!(k[0], "popular");
        assert_eq!(k[1], "aax", "then alphabetical");
        let _ = fs::remove_dir_all(&d);
    }

    fn reply(status: u16, retry_after: Option<&str>, body: &str) -> Reply {
        Reply {
            status,
            retry_after: retry_after.map(String::from),
            body: body.into(),
        }
    }

    #[test]
    fn every_status_gives_one_line_and_the_right_charge() {
        let err = |m: &str| json!({"error": {"message": m}}).to_string();
        let line = |r: &Reply| {
            let (reading, ms) = read_reply(r, 0.01);
            assert_eq!(ms, None);
            (reading.outcome.unwrap_err().reason(), reading.charge)
        };
        let (m, c) = line(&reply(400, None, &err("audio is longer than 10 minutes")));
        assert!(
            m.starts_with("Meta rejected the request: audio is longer than 10 minutes"),
            "{m}"
        );
        assert!(matches!(c, Charge::Free));
        for s in [401, 403] {
            let (m, c) = line(&reply(s, None, "{}"));
            assert!(
                m.contains("refused the API key") && m.contains(&s.to_string()),
                "{m}"
            );
            assert!(matches!(c, Charge::Free));
        }
        let (m, c) = line(&reply(
            402,
            None,
            r#"{"error":{"code":"billing_not_configured"}}"#,
        ));
        assert!(m.contains("billing") && matches!(c, Charge::Free), "{m}");
        let (m, c) = line(&reply(406, None, ""));
        assert!(m.contains("406") && matches!(c, Charge::Free), "{m}");
        let (m, c) = line(&reply(413, None, ""));
        assert!(
            m.contains("too large") && m.contains("32 MB") && matches!(c, Charge::Free),
            "{m}"
        );
        let (m, c) = line(&reply(429, Some("7"), &err("slow down")));
        assert!(
            m.contains("retry after 7 s") && m.contains("slow down") && matches!(c, Charge::Free),
            "{m}"
        );
        for s in [500, 502, 503] {
            let (m, c) = line(&reply(s, None, &err("boom")));
            assert!(m.contains("server error") && m.contains("boom"), "{m}");
            assert!(
                matches!(c, Charge::Estimate(x) if (x - 0.01).abs() < 1e-12),
                "the worst case is charged"
            );
        }
        let (m, c) = line(&reply(418, None, ""));
        assert!(m.contains("418") && matches!(c, Charge::Free), "{m}");
        // a 200 with no transcript, or no JSON, is billed as the worst case and refused
        for body in ["{}", "not json", r#"{"transcript": 5}"#] {
            let (m, c) = line(&reply(200, None, body));
            assert!(
                m.contains("without a transcript") && matches!(c, Charge::Estimate(_)),
                "{m}"
            );
        }
    }

    #[test]
    fn a_good_answer_is_billed_by_the_second_the_server_counted() {
        let body = json!({"sessionId": "s", "transcript": "login works now", "audioDurationMs": 2580, "turns": []}).to_string();
        let (reading, ms) = read_reply(&reply(200, None, &body), 0.01);
        assert_eq!(reading.outcome.unwrap(), "login works now");
        assert_eq!(ms, Some(2580));
        assert!(
            matches!(reading.charge, Charge::Usd(x) if (x - 3.0 * 0.18 / 3600.0).abs() < 1e-12),
            "3 s, rounded up"
        );
        // no length in the answer: the worst case; silence is an empty transcript, still a good answer
        let (reading, ms) = read_reply(&reply(200, None, r#"{"transcript": ""}"#), 0.01);
        assert_eq!((reading.outcome.unwrap(), ms), (String::new(), None));
        assert!(matches!(reading.charge, Charge::Estimate(_)));
    }

    #[test]
    fn the_help_says_where_the_audio_goes_and_what_else_is_sent() {
        let h = help();
        assert!(h.contains(PRIVACY), "{h}");
        assert!(
            h.contains("URL paths") && h.contains("--no-keywords") && h.contains(FFMPEG),
            "{h}"
        );
        assert_eq!(say(&["--help"]), 0);
        assert_eq!(say(&[]), 2);
        assert_eq!(say(&["--bogus", "a.wav"]), 2);
        assert_eq!(say(&["a.wav", "b.wav"]), 2);
    }

    /// The keyword measurement of docs/voice.md: every clip of a manifest, once without and once with the project's
    /// keywords, through the same `transcribe` that `say` uses. It costs money, so it is ignored. Run it by hand:
    ///   META_API_KEY=... TORPOR_MUSE_STATE=<dir> VOICE_MANIFEST=<manifest.json> VOICE_ROWS=<rows.jsonl> \
    ///   cargo test --bin torpor-muse -- --ignored --exact suggest::voice::tests::measure_keywords --nocapture
    #[test]
    #[ignore = "costs money: real transcriptions"]
    fn measure_keywords() {
        let get = |n: &str| std::env::var(n).unwrap_or_else(|_| panic!("set {n}"));
        // both are read later by `transcribe`: say so before any call, and never write to the default state dir
        for n in ["META_API_KEY", "TORPOR_MUSE_STATE"] {
            get(n);
        }
        let manifest: Vec<Value> =
            serde_json::from_str(&fs::read_to_string(get("VOICE_MANIFEST")).unwrap()).unwrap();
        // a clip and arm that already has a row is skipped: a run that stopped halfway goes on, nothing is asked twice
        let done: Vec<(String, String)> = fs::read_to_string(get("VOICE_ROWS"))
            .unwrap_or_default()
            .lines()
            .filter_map(|l| serde_json::from_str::<Value>(l).ok())
            .map(|r| {
                let s = |k: &str| r[k].as_str().unwrap_or("").to_string();
                (s("id"), s("arm"))
            })
            .collect();
        let mut rows = fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(get("VOICE_ROWS"))
            .unwrap();
        for clip in manifest {
            let app = Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("eval/apps")
                .join(clip["app"].as_str().unwrap());
            for arm in ["without", "with"] {
                if done.contains(&(clip["id"].as_str().unwrap().to_string(), arm.to_string())) {
                    continue;
                }
                let words = if arm == "with" {
                    keywords(&app)
                } else {
                    Vec::new()
                };
                // a call that fails (network, 5xx, rate limit) is made again 10 s later, up to 3 tries; a transcript that
                // came back is never asked for twice
                let mut tries = 0;
                let (text, ms) = loop {
                    tries += 1;
                    match transcribe(clip["file"].as_str().unwrap(), &words) {
                        Ok(h) => break h,
                        Err(e) if tries < 3 => {
                            eprintln!("{} {arm}: try {tries} failed: {e}", clip["id"]);
                            std::thread::sleep(std::time::Duration::from_secs(10));
                        }
                        Err(e) => panic!("{} {arm}: {e}", clip["id"]),
                    }
                };
                let row = json!({"id": clip["id"], "voice": clip["voice"], "app": clip["app"], "arm": arm,
                    "keywords": words, "transcript": text, "audioDurationMs": ms, "tries": tries});
                writeln!(rows, "{row}").unwrap();
                println!("{} {arm}: {text}", clip["id"]);
            }
        }
    }
}
