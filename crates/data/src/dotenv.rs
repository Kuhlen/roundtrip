//! `.env` files: upstream parse rules, line-preserving rewrite.

use std::collections::HashSet;
use std::fs;
use std::path::Path;

use domain::AppError;
use domain::environment::Variable;

use crate::collection_dir::write_text_atomic;
use crate::storage;

/// File order of each key's first line, last value wins (upstream HashMap insert);
/// missing file = empty (upstream).
pub(crate) fn parse(path: &Path) -> Vec<(String, String)> {
    let Ok(text) = fs::read_to_string(path) else {
        return Vec::new();
    };
    let mut out: Vec<(String, String)> = Vec::new();
    for (k, v) in text
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty() && !line.starts_with('#'))
        .filter_map(|line| line.split_once('='))
    {
        let (k, v) = (k.trim().to_owned(), unquote(v.trim()).to_owned());
        match out.iter_mut().find(|(key, _)| *key == k) {
            Some(entry) => entry.1 = v,
            None => out.push((k, v)),
        }
    }
    out
}

/// One line per entry: these would corrupt the file.
pub(crate) fn check(vars: &[&Variable], file: &Path) -> Result<(), AppError> {
    // folder kept for the secrets file so both cases read differently
    let label = match file.parent().and_then(Path::file_name) {
        Some(dir) if dir == ".apiark" => format!(".apiark/{}", file_name(file)),
        _ => file_name(file),
    };
    match vars
        .iter()
        .find(|v| v.key.contains('=') || v.key.starts_with('#') || v.value.contains(['\n', '\r']))
    {
        Some(v) => Err(AppError::Storage(format!(
            "{} can't be stored in {}",
            v.key, label
        ))),
        None => Ok(()),
    }
}

fn file_name(file: &Path) -> String {
    file.file_name().map_or_else(
        || file.display().to_string(),
        |n| n.to_string_lossy().into_owned(),
    )
}

/// A wanted key's first line gets its value, later duplicates and `drop`ped keys go,
/// every other line stays, unseen wanted keys are appended. No write when nothing changes.
pub(crate) fn rewrite(
    file: &Path,
    wanted: &[&Variable],
    drop: impl Fn(&str) -> bool,
) -> Result<(), AppError> {
    let text = match fs::read_to_string(file) {
        Ok(text) => text,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => String::new(),
        Err(e) => return Err(storage(file, e)),
    };
    // keep the file's own line endings
    let nl = if text.contains("\r\n") { "\r\n" } else { "\n" };
    let mut seen = HashSet::new();
    let mut out = String::new();
    for line in text.lines() {
        if let Some(key) = key_of(line) {
            if let Some(v) = wanted.iter().find(|v| v.key == key) {
                if seen.insert(key) {
                    out.push_str(&format!("{key}={}{nl}", quote(&v.value)));
                }
                continue;
            }
            if drop(key) {
                continue;
            }
        }
        out.push_str(line);
        out.push_str(nl);
    }
    for v in wanted.iter().filter(|v| !seen.contains(v.key.as_str())) {
        out.push_str(&format!("{}={}{nl}", v.key, quote(&v.value)));
    }
    if out.trim_end() == text.trim_end() {
        return Ok(());
    }
    let dir = file
        .parent()
        .ok_or_else(|| AppError::Storage(format!("{}: no folder", file.display())))?;
    ensure_ignored(dir)?;
    write_text_atomic(file, &out)
}

/// A `.env` file must never be committed.
fn ensure_ignored(dir: &Path) -> Result<(), AppError> {
    let file = dir.join(".gitignore");
    let text = match fs::read_to_string(&file) {
        Ok(text) => text,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => String::new(),
        Err(e) => return Err(storage(&file, e)),
    };
    if text.lines().any(|l| l.trim() == ".env") {
        return Ok(());
    }
    let sep = if text.is_empty() || text.ends_with('\n') {
        ""
    } else {
        "\n"
    };
    write_text_atomic(&file, &format!("{text}{sep}.env\n"))
}

fn key_of(line: &str) -> Option<&str> {
    let line = line.trim();
    if line.is_empty() || line.starts_with('#') {
        return None;
    }
    line.split_once('=').map(|(k, _)| k.trim())
}

// parse trims and strips one outer quote pair, so wrapping always reads back
fn quote(value: &str) -> String {
    if value != value.trim() || value.starts_with(['"', '\'']) {
        format!("\"{value}\"")
    } else {
        value.to_owned()
    }
}

// len >= 2: upstream panics on a lone quote
fn unquote(v: &str) -> &str {
    for q in ['"', '\''] {
        if v.len() >= 2 && v.starts_with(q) && v.ends_with(q) {
            return &v[1..v.len() - 1];
        }
    }
    v
}
