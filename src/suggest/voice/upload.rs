//! The upload: the multipart request, the call to the transcribe API and what its reply means.

use super::wav::read_clip;
use crate::ledger::{ASR_MODEL, Charge, Ledger, asr_cost};
use crate::suggest::client::{self, Bad, Key, Net, Reading, Reply};
use crate::suggest::{prompt, state};
use serde_json::{Value, json};
use std::time::{Duration, Instant};

const MAX_REPLY: u64 = 1_000_000;

pub(super) fn contains(hay: &[u8], needle: &[u8]) -> bool {
    hay.windows(needle.len()).any(|w| w == needle)
}

/// The multipart body (a `request` part and an `audio` part) and its boundary, which is not in the audio. The audio
/// part always has the same file name, never the user's.
pub(super) fn multipart(wav: &[u8], keywords: &[String]) -> (String, Vec<u8>) {
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

/// The same kind of call as `client::post`, with the audio as a multipart body.
fn post_audio(
    base: &client::Base,
    key: &Key,
    boundary: &str,
    body: &[u8],
    timeout: Duration,
) -> Result<Reply, Net> {
    let headers = [
        ("Accept", "application/json"),
        (
            "Content-Type",
            &format!("multipart/form-data; boundary={boundary}"),
        ),
    ];
    client::post_to(
        base,
        "/v1/asr/transcribe",
        key,
        &headers,
        body,
        MAX_REPLY,
        timeout,
    )
}

/// One answer, read: what it cost, what was heard, and the audio length the server billed (when it said).
/// The error statuses are the writer's, plus 406 and 413.
pub(super) fn read_reply(r: &Reply, worst: f64) -> (Reading, Option<u64>) {
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
pub(super) fn transcribe(clip: &str, words: &[String]) -> Result<(String, u64), String> {
    let (wav, header_ms) = read_clip(clip)?;
    let base = client::api_base()?;
    let http = if base.loopback {
        crate::suggest::seconds("TORPOR_MUSE_HTTP_TIMEOUT_S")
            .unwrap_or(crate::suggest::HTTP_TIMEOUT)
    } else {
        crate::suggest::HTTP_TIMEOUT
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
