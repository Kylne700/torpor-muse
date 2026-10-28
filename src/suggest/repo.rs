//! What the writer may read of the project: the root, the start scripts, and a capped, redacted excerpt of
//! allowlisted files. Nothing here is sent anywhere; `mod.rs` sends the result. Owner: S3b-writer.
mod excerpt;
mod redact;
mod root;
#[cfg(test)]
mod tests;

pub(super) use excerpt::read_capped;
pub use excerpt::{allowed_files, cut, excerpt};
pub(super) use redact::MASK;
pub use redact::redact;
pub use root::{is_synthetic, resolve_root, start_choices};
