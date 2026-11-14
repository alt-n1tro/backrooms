//! Golden-hash test: the world must not change by accident.
//!
//! The expected table lives in `tests/golden/v1.txt`. This test must pass,
//! with that same file, on every platform (CI runs it on x86_64 and ARM64,
//! in debug and release builds).
//!
//! While v1 is a draft, an intended change to the generator is accepted with
//!
//! ```text
//! BACKROOMS_BLESS=1 cargo test -p backrooms-core --test golden
//! ```
//!
//! and the diff of the golden file is reviewed like any other change. Once v1
//! ships, set `V1_FROZEN` to true: blessing is then refused, for good.

use std::path::PathBuf;

use backrooms_core::generator::v1::golden;

const V1_FROZEN: bool = false;

#[test]
fn v1_matches_golden_hashes() {
    // CARGO_MANIFEST_DIR is the crate's directory at compile time, so the
    // test finds its file no matter where `cargo test` is run from.
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/golden/v1.txt");
    let actual = golden::table().to_string();

    if std::env::var_os("BACKROOMS_BLESS").is_some() {
        if V1_FROZEN {
            panic!("generator v1 has shipped; its output can no longer be re-blessed");
        }
        std::fs::write(&path, &actual).expect("write golden file");
        eprintln!("blessed {}", path.display());
        return;
    }

    let expected = std::fs::read_to_string(&path).expect("read golden file");
    // Compare line by line so a failure names the chunk, and so that a
    // checkout with Windows line endings still passes.
    let mut mismatches = Vec::new();
    let (mut want, mut got) = (expected.lines(), actual.lines());
    loop {
        match (want.next(), got.next()) {
            (None, None) => break,
            (w, g) if w == g => {}
            (w, g) => mismatches.push(format!("  expected: {}\n  actual:   {}", w.unwrap_or("-"), g.unwrap_or("-"))),
        }
    }
    assert!(
        mismatches.is_empty(),
        "generator v1 output changed in {} line(s):\n{}\n\
         If this change is intended and v1 has not shipped, re-bless (see the top of this file).",
        mismatches.len(),
        mismatches.join("\n")
    );
}
