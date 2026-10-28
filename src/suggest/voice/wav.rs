//! Checking the clip before it is sent: a plain WAV of the right format, length and size.

use super::FFMPEG;
use std::fs;

/// Limits of the API: more bytes get a 413, more audio a 400.
pub(super) const MAX_FILE: u64 = 32_000_000;
const MAX_MS: u64 = 600_000;

/// The length in ms of a WAV the API takes, or why it will not take it: PCM, mono, 16-bit, 16000 or 24000 Hz, at
/// most 10 minutes. Extra chunks (LIST and so on) are skipped; a data chunk that claims more than the file holds
/// counts as what is there.
pub(super) fn check_wav(b: &[u8]) -> Result<u64, String> {
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
                        "the clip is {} s long; the limit is 10 minutes (600 s)",
                        ms.div_ceil(1000)
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
pub(super) fn read_clip(path: &str) -> Result<(Vec<u8>, u64), String> {
    let hint = |why: String| format!("{why}. To convert it: {FFMPEG}");
    let meta = fs::metadata(path).map_err(|e| format!("cannot open {path}: {e}"))?;
    if !meta.is_file() {
        return Err(format!("{path} is not a file"));
    }
    if meta.len() > MAX_FILE {
        let tenths = meta.len().div_ceil(100_000); // rounded up: never "32 MB" about a file over 32 MB
        return Err(hint(format!(
            "{path} is {}.{} MB; the limit is 32 MB",
            tenths / 10,
            tenths % 10
        )));
    }
    let bytes = fs::read(path).map_err(|e| format!("cannot read {path}: {e}"))?;
    let ms = check_wav(&bytes).map_err(|why| hint(format!("{path}: {why}")))?;
    Ok((bytes, ms))
}
