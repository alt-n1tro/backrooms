//! Enforces the "integers only" rule by reading the crate's own source.
//!
//! Clippy's `float_arithmetic` lint (denied in Cargo.toml) catches float
//! maths; this catches the types and literals themselves, including in code
//! clippy never sees because of a `cfg`.

use std::fs;
use std::path::{Path, PathBuf};

fn rust_files(dir: &Path, out: &mut Vec<PathBuf>) {
    for entry in fs::read_dir(dir).expect("read source directory") {
        let path = entry.expect("read directory entry").path();
        if path.is_dir() {
            rust_files(&path, out);
        } else if path.extension().is_some_and(|ext| ext == "rs") {
            out.push(path);
        }
    }
}

/// The code part of a line: everything before a `//` comment.
fn code(line: &str) -> &str {
    line.split("//").next().unwrap_or("")
}

fn is_ident(c: char) -> bool {
    c.is_ascii_alphanumeric() || c == '_'
}

/// True if `code` mentions `f32`/`f64` as a whole word or literal suffix, or
/// contains a float literal such as `1.5`.
fn mentions_float(code: &str) -> bool {
    let chars: Vec<char> = code.chars().collect();
    for i in 0..chars.len() {
        let rest: String = chars[i..].iter().take(3).collect();
        let starts_word = i == 0 || !is_ident(chars[i - 1]);
        if (rest == "f32" || rest == "f64") && !chars.get(i + 3).is_some_and(|&c| is_ident(c)) {
            // Catches both the bare type and a suffix like `1f32`.
            if starts_word || chars[i - 1].is_ascii_digit() {
                return true;
            }
        }
        // digits '.' digit is a float literal. Ranges are `..`, and methods
        // and fields have a letter after the dot. Tuple fields (`t.0.1`) are
        // told apart by what comes before the digits: a dot or a name.
        if chars[i] == '.' && chars.get(i + 1).is_some_and(|c| c.is_ascii_digit()) {
            let mut start = i;
            while start > 0 && (chars[start - 1].is_ascii_digit() || chars[start - 1] == '_') {
                start -= 1;
            }
            let has_digits = start < i;
            let after_name_or_dot = start > 0 && (is_ident(chars[start - 1]) || chars[start - 1] == '.');
            if has_digits && !after_name_or_dot {
                return true;
            }
        }
    }
    false
}

#[test]
fn detector_works() {
    for float in ["let x: f32 = y;", "1.5", "x as f64", "2f32", "foo(0.25)", "1_000.0"] {
        assert!(mentions_float(float), "missed: {float}");
    }
    for fine in ["a.0 + b.1", "0..16", "buf32", "x.wrapping_add(1)", "t.0.0", "0x1.max(2)"] {
        assert!(!mentions_float(fine), "false alarm: {fine}");
    }
}

#[test]
fn core_source_has_no_floats() {
    let mut files = Vec::new();
    rust_files(&PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("src"), &mut files);
    assert!(files.len() >= 8, "source files not found");

    let mut offences = Vec::new();
    for path in files {
        for (number, line) in fs::read_to_string(&path).expect("read source file").lines().enumerate() {
            if mentions_float(code(line)) {
                offences.push(format!("{}:{}: {}", path.display(), number + 1, line.trim()));
            }
        }
    }
    assert!(offences.is_empty(), "floats in backrooms-core:\n{}", offences.join("\n"));
}
