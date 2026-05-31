use std::fs;
use std::path::{Path, PathBuf};

#[test]
fn error_code_messages_are_not_empty_or_placeholders() {
    let root = workspace_root();
    let mut checked = 0usize;
    let mut failures = Vec::new();

    for path in rust_sources(&root.join("crates")) {
        let source = fs::read_to_string(&path).expect("read Rust source");
        for attr in error_attrs(&source) {
            if !attr.contains("E_") {
                continue;
            }
            checked += 1;
            if looks_placeholder(&attr) {
                failures.push(format!("{}: {attr}", path.display()));
            }
        }
    }

    assert!(checked > 0, "audit should inspect at least one E_* message");
    assert!(
        failures.is_empty(),
        "E_* messages must be actionable, not empty/placeholders:\n{}",
        failures.join("\n")
    );
}

#[test]
fn appendix_b_warning_rows_have_actionable_messages_when_spec_is_available() {
    let Some(path) = appendix_b_path() else {
        eprintln!("skipping appendix-b warning audit: spec checkout not found");
        return;
    };
    let source = fs::read_to_string(path).expect("read appendix-b warnings");
    let mut checked = 0usize;
    let mut failures = Vec::new();

    for line in source.lines().filter(|line| line.contains("| `W_")) {
        checked += 1;
        if looks_warning_placeholder(line) || line.len() < 48 {
            failures.push(line.to_string());
        }
    }

    assert!(
        checked > 0,
        "warning audit should inspect at least one appendix-b warning row"
    );
    assert!(
        failures.is_empty(),
        "warning messages must be actionable, not empty/placeholders:\n{}",
        failures.join("\n")
    );
}

fn workspace_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("examples crate lives under the workspace root")
        .to_path_buf()
}

fn appendix_b_path() -> Option<PathBuf> {
    if let Ok(dir) = std::env::var("VERBREEL_SPEC_DIR") {
        return Some(
            PathBuf::from(dir)
                .join("spec")
                .join("commands")
                .join("appendix-b-warnings.md"),
        );
    }

    let mut dir = workspace_root();
    loop {
        let candidate = dir
            .join("verbreel-spec")
            .join("spec")
            .join("commands")
            .join("appendix-b-warnings.md");
        if candidate.exists() {
            return Some(candidate);
        }
        if !dir.pop() {
            return None;
        }
    }
}

fn rust_sources(dir: &Path) -> Vec<PathBuf> {
    let mut out = Vec::new();
    collect_rust_sources(dir, &mut out);
    out.sort();
    out
}

fn collect_rust_sources(dir: &Path, out: &mut Vec<PathBuf>) {
    for entry in fs::read_dir(dir).expect("read source directory") {
        let path = entry.expect("directory entry").path();
        if path.is_dir() {
            collect_rust_sources(&path, out);
        } else if path.extension().and_then(|ext| ext.to_str()) == Some("rs") {
            out.push(path);
        }
    }
}

fn error_attrs(source: &str) -> Vec<String> {
    let mut attrs = Vec::new();
    let mut current: Option<String> = None;

    for line in source.lines() {
        let trimmed = line.trim();
        if let Some(attr) = &mut current {
            attr.push(' ');
            attr.push_str(trimmed);
            if trimmed.ends_with(")]") {
                attrs.push(current.take().expect("current attr exists"));
            }
            continue;
        }
        if trimmed.starts_with("#[error(") {
            if trimmed.ends_with(")]") {
                attrs.push(trimmed.to_string());
            } else {
                current = Some(trimmed.to_string());
            }
        }
    }

    attrs
}

fn looks_placeholder(attr: &str) -> bool {
    let lower = attr.to_lowercase();
    if ["todo", "tbd", "placeholder", "not yet implemented"]
        .iter()
        .any(|needle| lower.contains(needle))
    {
        return true;
    }

    let Some(start) = attr.find("E_") else {
        return false;
    };
    let after_code = attr[start..]
        .split_once(|c: char| !(c.is_ascii_uppercase() || c.is_ascii_digit() || c == '_'))
        .map_or("", |(_, rest)| rest)
        .trim_matches(|c: char| {
            c.is_whitespace() || matches!(c, ':' | '-' | '—' | '`' | '"' | ')')
        });

    after_code.len() < 8
}

fn looks_warning_placeholder(row: &str) -> bool {
    let lower = row.to_lowercase();
    ["todo", "tbd", "not yet implemented"]
        .iter()
        .any(|needle| lower.contains(needle))
}
