use crate::domain::*;
use std::collections::BTreeMap;
#[derive(Debug, Clone)]
pub struct DesktopEntry {
    pub id: String,
    pub name: String,
    pub exec: String,
    pub icon: Option<String>,
    pub terminal: bool,
}
pub fn parse_desktop(id: &str, bytes: &[u8]) -> AppResult<Option<DesktopEntry>> {
    if bytes.len() > 1_048_576 {
        return Err(AppError::new(
            ErrorCode::InvalidInput,
            "Lanzador demasiado grande",
        ));
    }
    let text = std::str::from_utf8(bytes)
        .map_err(|_| AppError::new(ErrorCode::InvalidInput, "Lanzador no UTF-8"))?;
    let mut fields = BTreeMap::new();
    let mut active = false;
    for line in text.lines() {
        let line = line.trim();
        if line.starts_with('[') {
            active = line == "[Desktop Entry]";
            continue;
        }
        if !active || line.starts_with('#') {
            continue;
        }
        if let Some((key, value)) = line.split_once('=') {
            fields.insert(key.trim(), value.trim());
        }
    }
    if fields.get("Hidden") == Some(&"true") || fields.get("NoDisplay") == Some(&"true") {
        return Ok(None);
    }
    if fields.get("Type").is_some_and(|t| *t != "Application") {
        return Ok(None);
    }
    let name = fields
        .get("Name[es]")
        .or_else(|| fields.get("Name"))
        .copied()
        .unwrap_or("");
    let exec = fields.get("Exec").copied().unwrap_or("");
    if name.is_empty() || exec.is_empty() {
        return Ok(None);
    }
    let decode = |s: &str| {
        s.replace("\\s", " ")
            .replace("\\n", "\n")
            .replace("\\t", "\t")
            .replace("\\r", "\r")
            .replace("\\\\", "\\")
    };
    Ok(Some(DesktopEntry {
        id: id.into(),
        name: decode(name),
        exec: exec.into(),
        icon: fields.get("Icon").map(|s| s.to_string()),
        terminal: fields.get("Terminal") == Some(&"true"),
    }))
}
pub fn exec_words(exec: &str) -> AppResult<Vec<String>> {
    let mut words = Vec::new();
    let mut word = String::new();
    let mut quoted = false;
    let mut escape = false;
    for c in exec.chars() {
        if escape {
            word.push(c);
            escape = false;
            continue;
        }
        if c == '\\' {
            escape = true;
            continue;
        }
        if c == '"' {
            quoted = !quoted;
            continue;
        }
        if c.is_whitespace() && !quoted {
            if !word.is_empty() {
                words.push(std::mem::take(&mut word));
            }
        } else {
            word.push(c);
        }
    }
    if quoted || escape {
        return Err(AppError::new(ErrorCode::InvalidInput, "Exec mal formado"));
    }
    if !word.is_empty() {
        words.push(word);
    }
    Ok(words)
}
