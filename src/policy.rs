use crate::{domain::*, paths::Paths};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    path::{Path, PathBuf},
};
const BUILTIN: &str = include_str!("../catalog/managed-exclusions.json");
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
struct Rule {
    aliases: Vec<String>,
    names: Vec<String>,
    desktop_ids: Vec<String>,
    paths: Vec<String>,
}
#[derive(Debug, Clone)]
pub struct Policy {
    pub coverage: Coverage,
    home: PathBuf,
    rules: BTreeMap<String, Rule>,
    source_targets: Vec<String>,
    source_hash: String,
}
impl Policy {
    pub fn load(updater_source: &Path, catalog: &Path) -> AppResult<Self> {
        let rules: BTreeMap<String, Rule> = serde_json::from_str(BUILTIN)?;
        let source = fs::read_to_string(updater_source).ok();
        let source_hash = format!(
            "{:x}",
            Sha256::digest(source.as_deref().unwrap_or("").as_bytes())
        );
        let parsed = source.as_deref().and_then(|s| parse_targets(s).ok());
        let source_targets = parsed.clone().unwrap_or_default();
        let known = rules.keys().cloned().collect::<BTreeSet<_>>();
        let observed = source_targets.iter().cloned().collect::<BTreeSet<_>>();
        let catalog_good = match fs::read(catalog) {
            Ok(bytes) => {
                serde_json::from_slice::<BTreeMap<String, Rule>>(&bytes).is_ok_and(|r| r == rules)
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => true,
            Err(_) => false,
        };
        let coverage = if parsed.is_some() && known == observed && catalog_good {
            Coverage::Verified
        } else {
            Coverage::NeedsReview {
                reason:
                    "Cambió la cobertura de updater.py o su catálogo; revisar antes de actualizar"
                        .into(),
                unknown: observed.difference(&known).cloned().collect(),
            }
        };
        let home = updater_source
            .ancestors()
            .nth(4)
            .unwrap_or(Path::new("/nonexistent"))
            .to_path_buf();
        Ok(Self {
            coverage,
            home,
            rules,
            source_targets,
            source_hash,
        })
    }
    pub fn for_paths(paths: &Paths) -> AppResult<Self> {
        Self::load(
            &paths.home.join(".local/share/ai-updaters/updater.py"),
            &paths.plugin.join("catalog/managed-exclusions.json"),
        )
    }
    pub fn protected_package(&self, name: &str) -> bool {
        self.rules
            .values()
            .any(|r| r.aliases.iter().any(|a| a == name))
    }
    pub fn protected_path(&self, path: &Path) -> bool {
        self.rules
            .values()
            .any(|r| r.paths.iter().any(|p| path.starts_with(self.home.join(p))))
            || path.file_name().and_then(|n| n.to_str()).is_some_and(|n| {
                self.rules
                    .values()
                    .any(|r| r.aliases.iter().any(|a| a == n))
            })
    }
    pub fn excluded(&self, app: &App) -> bool {
        if app
            .components
            .iter()
            .all(|t| matches!(t.locator, Locator::Web { .. }))
        {
            return false;
        }
        if app.components.iter().all(|t|matches!(&t.locator,Locator::Repo{package}|Locator::Aur{package,..}if package=="hermes-desktop")){return false;}
        self.rules.values().any(|r| {
            r.names.iter().any(|n| n.eq_ignore_ascii_case(&app.name))
                || r.desktop_ids.iter().any(|id| app.desktop_ids.contains(id))
        }) || app.components.iter().any(|t| match &t.locator {
            Locator::Repo { package } | Locator::Aur { package, .. } => {
                self.protected_package(package)
            }
            Locator::Unknown { path: Some(p) } | Locator::TelegramLocal { root: p } => {
                self.protected_path(p)
            }
            _ => false,
        })
    }
    pub fn visible(&self, apps: &[App]) -> Vec<App> {
        apps.iter().filter(|a| !self.excluded(a)).cloned().collect()
    }
    pub fn selected(&self, apps: &[App], config: &UserConfig) -> AppResult<Vec<AppId>> {
        Ok(self
            .visible(apps)
            .iter()
            .filter(|a| {
                if config.initialized {
                    config.selected.contains(&a.id)
                } else {
                    a.suggested
                }
            })
            .map(|a| a.id.clone())
            .collect::<BTreeSet<_>>()
            .into_iter()
            .collect())
    }
    pub fn digest(&self) -> String {
        format!(
            "{:x}",
            Sha256::digest(
                serde_json::to_vec(&(
                    &self.rules,
                    &self.source_targets,
                    &self.source_hash,
                    &self.home
                ))
                .unwrap_or_default()
            )
        )
    }
    pub fn filter_report(&self, report: &mut Report, config: &UserConfig) -> AppResult<()> {
        report.apps = self.visible(&report.apps);
        report.selected = self.selected(&report.apps, config)?;
        let targets = report
            .apps
            .iter()
            .flat_map(|a| a.components.iter().map(|t| &t.id))
            .collect::<BTreeSet<_>>();
        report.checks.retain(|c| targets.contains(&c.target_id));
        report.coverage = self.coverage.clone();
        Ok(())
    }
}
fn parse_targets(source: &str) -> AppResult<Vec<String>> {
    let assignments = source
        .lines()
        .filter(|line| {
            line.trim_start()
                .strip_prefix("TARGETS")
                .is_some_and(|rest| {
                    ["=", "+=", "-=", "*="]
                        .iter()
                        .any(|op| rest.trim_start().starts_with(op))
                })
        })
        .count();
    if assignments != 1 {
        return Err(AppError::new(
            ErrorCode::InvalidInput,
            "TARGETS no es una sola declaración literal",
        ));
    }
    let mut offset = 0;
    let mut tail = None;
    for line in source.split_inclusive('\n') {
        let line_start = line.trim_start();
        if let Some(rest) = line_start.strip_prefix("TARGETS")
            && let Some(value) = rest.trim_start().strip_prefix('=')
        {
            tail = Some(&source[offset + line.len() - value.len()..]);
            break;
        }
        offset += line.len();
    }
    let tail = tail
        .ok_or_else(|| AppError::new(ErrorCode::InvalidInput, "TARGETS no encontrado"))?
        .trim_start();
    if !tail.starts_with('(') {
        return Err(AppError::new(ErrorCode::InvalidInput, "TARGETS no literal"));
    }
    let mut chars = tail[1..].char_indices().peekable();
    let mut values = Vec::new();
    let mut expect_value = true;
    while let Some((i, c)) = chars.next() {
        if c.is_whitespace() {
            continue;
        }
        if c == ')' {
            let after = tail[i + 2..].split('\n').next().unwrap_or("").trim();
            if !after.is_empty() && !after.starts_with('#') {
                return Err(AppError::new(
                    ErrorCode::InvalidInput,
                    "Expresión TARGETS no admitida",
                ));
            }
            return Ok(values);
        }
        if c == ',' && !expect_value {
            expect_value = true;
            continue;
        }
        if !expect_value || !matches!(c, '\'' | '"') {
            return Err(AppError::new(ErrorCode::InvalidInput, "TARGETS no literal"));
        }
        let quote = c;
        let mut value = String::new();
        let mut closed = false;
        for (_, ch) in chars.by_ref() {
            if ch == quote {
                closed = true;
                break;
            }
            if !ch.is_ascii_alphanumeric() && !matches!(ch, '-' | '_') {
                return Err(AppError::new(ErrorCode::InvalidInput, "Destino inválido"));
            }
            value.push(ch);
        }
        if !closed || value.is_empty() || value.len() > 80 {
            return Err(AppError::new(ErrorCode::InvalidInput, "Destino inválido"));
        }
        values.push(value);
        expect_value = false;
    }
    Err(AppError::new(ErrorCode::InvalidInput, "TARGETS incompleto"))
}
