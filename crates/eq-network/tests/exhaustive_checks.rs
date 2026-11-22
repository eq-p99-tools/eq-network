//! Keeps every client-generation and server-type decision compiler-checked.
//!
//! `GameDialect` and `ServerProtocol` are deliberately exhaustive (see
//! API-CONTRACT.md), so a new variant breaks every `match` on them. A
//! comparison (`==`, `!=`, `matches!`, `if let`) on one variant would not
//! break: the new generation or server type would silently take the other
//! branch. This test finds such comparisons in production code across the
//! workspace. A decision that reads best as a yes/no is a named predicate on
//! the enum whose body is an exhaustive `match`, like `is_stock()`.

use std::{
    fs,
    path::{Path, PathBuf},
};

/// The enums whose variants every decision must name.
const TYPES: &[&str] = &["GameDialect", "ServerProtocol"];

/// The files that define those enums, where `Self::` names a variant too.
const DEFINING: &[&str] = &["eq-network-game/src/lib.rs", "eq-network/src/client.rs"];

/// What a reported line should become.
const ADVICE: &str = "write an exhaustive `match` naming every variant, or a named \
                      predicate on the enum whose body is one";

/// The comparisons on one variant in `line` that a new variant of one of
/// `types` would pass silently.
fn silent_check(line: &str, types: &[&str]) -> bool {
    let code = line.split("//").next().unwrap_or_default();
    types.iter().any(|name| {
        let path = format!("{name}::");
        code.match_indices(&path).any(|(at, _)| {
            // The whole path, with any crate or module prefix.
            let start = code[..at]
                .trim_end_matches(|c: char| c.is_alphanumeric() || c == '_' || c == ':')
                .len();
            if start < at && !code[start..at].ends_with("::") {
                // Part of a longer name, such as `MyGameDialect::`.
                return false;
            }
            let before = code[..start].trim_end();
            let rest = &code[at + path.len()..];
            let variant_end = rest
                .find(|c: char| !(c.is_alphanumeric() || c == '_'))
                .unwrap_or(rest.len());
            let after = rest[variant_end..].trim_start();
            before.ends_with("==")
                || before.ends_with("!=")
                || after.starts_with("==")
                || after.starts_with("!=")
                || before.ends_with("let")
                || code[..at].contains("matches!(")
        })
    })
}

/// The reported lines of one source file: file-relative line numbers and
/// text, skipping any item under `#[cfg(test)]`.
fn scan(text: &str, types: &[&str]) -> Vec<(usize, String)> {
    let mut found = Vec::new();
    let mut lines = text.lines().enumerate();
    while let Some((index, line)) = lines.next() {
        if line.trim() == "#[cfg(test)]" {
            skip_item(&mut lines);
            continue;
        }
        if silent_check(line, types) {
            found.push((index + 1, line.trim().to_owned()));
        }
    }
    found
}

/// Skips the item after a `#[cfg(test)]`: one line ending in `;`, or a
/// braced block up to its closing brace.
fn skip_item<'a>(lines: &mut impl Iterator<Item = (usize, &'a str)>) {
    let mut depth = 0_i64;
    let mut opened = false;
    for (_, line) in lines {
        let code = line.split("//").next().unwrap_or_default();
        for c in code.chars() {
            match c {
                '{' => {
                    depth += 1;
                    opened = true;
                }
                '}' => depth -= 1,
                _ => {}
            }
        }
        if (opened && depth <= 0) || (!opened && code.trim_end().ends_with(';')) {
            return;
        }
    }
}

/// Whether a path holds test code only, by this workspace's naming.
fn test_file(path: &Path) -> bool {
    let name = path
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or_default();
    name == "tests.rs"
        || name.ends_with("_tests.rs")
        || path.components().any(|part| part.as_os_str() == "tests")
}

fn sources(directory: &Path, into: &mut Vec<PathBuf>) {
    for entry in fs::read_dir(directory).unwrap().map(Result::unwrap) {
        let path = entry.path();
        if path.is_dir() {
            sources(&path, into);
        } else if path.extension().is_some_and(|e| e == "rs") && !test_file(&path) {
            into.push(path);
        }
    }
}

#[test]
fn every_generation_and_server_type_decision_is_exhaustive() {
    let crates = Path::new(env!("CARGO_MANIFEST_DIR")).join("..");
    let mut files = Vec::new();
    for member in fs::read_dir(&crates).unwrap().map(Result::unwrap) {
        let source = member.path().join("src");
        if source.is_dir() {
            sources(&source, &mut files);
        }
    }
    assert!(files.len() > 100, "found only {} source files", files.len());
    let mut reports = Vec::new();
    for file in files {
        let relative = file.strip_prefix(&crates).unwrap();
        let mut types = TYPES.to_vec();
        if DEFINING
            .iter()
            .any(|defining| relative == Path::new(defining))
        {
            types.push("Self");
        }
        let text = fs::read_to_string(&file).unwrap();
        for (line, code) in scan(&text, &types) {
            reports.push(format!("crates/{}:{line}: {code}", relative.display()));
        }
    }
    assert!(
        reports.is_empty(),
        "a new client generation or server type would pass these silently; {ADVICE}:\n{}",
        reports.join("\n")
    );
}

#[test]
fn the_guard_catches_each_form_and_skips_tests() {
    let caught = [
        "if dialect == GameDialect::Titanium {",
        "if dialect != GameDialect::EqMac {",
        "if GameDialect::EqMac == dialect {",
        "let titanium = matches!(dialect, GameDialect::Titanium);",
        "if let GameDialect::EqMac = dialect {",
        "let GameDialect::Titanium = dialect else { return };",
        "local: protocol == eq_network::client::ServerProtocol::EqEmu,",
    ];
    for line in caught {
        assert_eq!(scan(line, TYPES).len(), 1, "{line}");
    }
    assert_eq!(scan("matches!(self, Self::Titanium)", &["Self"]).len(), 1);
    let passed = [
        "GameDialect::Titanium => true,",
        "match GameDialect::from(protocol) {",
        "dialect: GameDialect::EqMac,",
        "// if dialect == GameDialect::Titanium {",
        "if mode == Mode::Titanium {",
        "GameDialect::Titanium | GameDialect::EqMac => matches!(class, 2..=6),",
    ];
    for line in passed {
        assert_eq!(scan(line, TYPES), [], "{line}");
    }
    let with_tests = "fn rule(d: GameDialect) -> bool {\n    match d {\n        GameDialect::Titanium => true,\n        GameDialect::EqMac => false,\n    }\n}\n\n#[cfg(test)]\nmod named_tests;\n\nfn other() -> bool { d == GameDialect::EqMac }\n\n#[cfg(test)]\nmod tests {\n    fn t() {\n        assert!(d == GameDialect::EqMac);\n    }\n}\n";
    assert_eq!(
        scan(with_tests, TYPES),
        [(
            11,
            "fn other() -> bool { d == GameDialect::EqMac }".to_owned()
        )]
    );
}
