//! The one-line install runs install.sh; the README INSTALL block runs the same script from a here-document
//! (`sh -eu <<'EOF'` ... `EOF`). scripts/cleanroom.sh tests the block at release, so install.sh must be that
//! here-document's text byte for byte: then the one line installs the same files with the same checksum checks.
use std::fs;
use std::path::Path;

/// The text after the first line of `text` that starts with `open` and before the next line that is exactly `close`,
/// every line with its newline (what a here-document hands to `sh`).
fn between<'a>(text: &'a str, open: &str, close: &str) -> &'a str {
    let mut from = None;
    let mut at = 0;
    for line in text.split_inclusive('\n') {
        match from {
            None if line.starts_with(open) => from = Some(at + line.len()),
            Some(start) if line.trim_end_matches('\n') == close => return &text[start..at],
            _ => {}
        }
        at += line.len();
    }
    panic!("no line starting {open:?} followed by a line {close:?}");
}

#[test]
fn install_sh_is_the_script_the_readme_install_block_runs() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let readme = fs::read_to_string(root.join("README.md")).unwrap();
    let section = between(&readme, "<!-- INSTALL:start -->", "<!-- INSTALL:end -->");
    let block = between(section, "```sh", "```");
    let script = between(block, "sh -eu <<'EOF'", "EOF");
    assert!(
        !script.is_empty(),
        "the README INSTALL block has an empty here-document"
    );
    let install = fs::read(root.join("install.sh")).unwrap();
    assert!(
        install == script.as_bytes(),
        "install.sh is not the text between `sh -eu <<'EOF'` and `EOF` in the README INSTALL block: change both together"
    );
}
