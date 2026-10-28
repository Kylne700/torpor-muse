//! Tests of the voice route: the clip checks, the keywords, the upload parts and the replies.

use super::keywords::{url_paths, words};
use super::upload::{contains, multipart, read_reply};
use super::wav::{MAX_FILE, check_wav, read_clip};
use super::*;
use crate::ledger::Charge;
use crate::suggest::client::Reply;
use serde_json::{Value, json};
use std::fs;
use std::io::Write;
use std::path::Path;

/// A WAV with `data` bytes of audio and the given format; `extra` is a chunk put before `data`.
fn wav_with(tag: u16, channels: u16, rate: u32, bits: u16, data: usize, extra: &[u8]) -> Vec<u8> {
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
    no(
        &good(16_000, 600_001),
        "601 s long; the limit is 10 minutes",
    );
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
    assert!(
        e.contains("32.1 MB; the limit is 32 MB") && e.contains(FFMPEG),
        "{e}"
    );
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

#[test]
fn a_multibyte_character_in_a_template_literal_is_not_cut_in_half() {
    // QA round 4: the look-ahead for `${` sliced the text at a byte inside the character and panicked
    let code = "// the route `/api/\u{2026}` is live\nfetch(`/n/\u{e9}t\u{e9}/${id}`);\n";
    assert_eq!(url_paths(code), ["/api/\u{2026}", "/n/\u{e9}t\u{e9}/"]);
    let d = project("utf8", &[("a.rs", code)]);
    assert_eq!(keywords(&d), ["api"]);
    let _ = fs::remove_dir_all(&d);
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
    fs::canonicalize(&d).unwrap() // the root a run gets is canonical; macOS temp dirs sit behind a symlink
}

#[test]
fn keywords_are_the_projects_own_words_the_same_every_time() {
    let d = project(
        "kw",
        &[
            (
                "server.js",
                "app.get('/api/login', a);\napp.post('/api/notes', b);\nconst k = '/hook/key-0123456789abcdef0123456789abcdef';",
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
            // a call that fails, whatever the error, is made again 10 s later, up to 3 tries in all and then the run
            // stops; a transcript that came back is never asked for twice
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
