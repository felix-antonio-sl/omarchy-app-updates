use crate::alpm::PackageFacts;
use std::{
    collections::HashMap,
    path::{Path, PathBuf},
};
pub struct Ownership<'a> {
    files: HashMap<&'a str, &'a PackageFacts>,
}
impl<'a> Ownership<'a> {
    pub fn new(facts: &'a [PackageFacts]) -> Self {
        Self {
            files: facts
                .iter()
                .flat_map(|p| p.files.iter().map(move |f| (f.as_str(), p)))
                .collect(),
        }
    }
    pub fn package(&self, path: &Path) -> Option<&'a PackageFacts> {
        self.files
            .get(path.to_string_lossy().trim_start_matches('/'))
            .copied()
    }
}
pub fn executable(token: &str, home: &Path) -> PathBuf {
    let p = PathBuf::from(token);
    if p.is_absolute() {
        return p.canonicalize().unwrap_or(p);
    }
    for dir in [
        home.join(".local/bin"),
        home.join(".cargo/bin"),
        PathBuf::from("/usr/bin"),
        PathBuf::from("/usr/local/bin"),
    ] {
        let p = dir.join(token);
        if p.is_file() {
            return p.canonicalize().unwrap_or(p);
        }
    }
    PathBuf::from("/usr/bin").join(token)
}

pub fn application_token(words: &[String]) -> &str {
    let Some(first) = words.first() else {
        return "";
    };
    let base = Path::new(first)
        .file_name()
        .and_then(|s| s.to_str())
        .unwrap_or("");
    if base.starts_with("python")
        || matches!(
            base,
            "sh" | "bash" | "zsh" | "fish" | "ruby" | "perl" | "node"
        )
    {
        return words
            .get(1)
            .filter(|s| !s.starts_with('-'))
            .map(String::as_str)
            .unwrap_or("");
    }
    if base.starts_with("omarchy-") {
        return "";
    }
    first
}
pub fn program_words(words: &[String]) -> &[String] {
    if words.is_empty() {
        return words;
    }
    let base = Path::new(&words[0])
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("");
    match base {
        "env" => {
            let n = words
                .iter()
                .skip(1)
                .take_while(|w| w.contains('=') || w.starts_with('-'))
                .count();
            program_words(&words[(n + 1).min(words.len())..])
        }
        "uwsm" if words.get(1).is_some_and(|w| w == "app") => {
            if let Some(i) = words.iter().position(|w| w == "--") {
                program_words(&words[i + 1..])
            } else {
                program_words(&words[2..])
            }
        }
        "xdg-terminal-exec" => program_words(&words[1..]),
        _ => words,
    }
}
