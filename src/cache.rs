use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};
use std::path::PathBuf;

use crate::fixtures::Fixture;

fn cache_dir() -> PathBuf {
    let home = std::env::var("HOME").unwrap_or_else(|_| "/tmp".into());
    PathBuf::from(home).join(".cache/pytest-fixtures-lsp")
}

fn cache_path(root_dir: &str) -> PathBuf {
    let mut hasher = DefaultHasher::new();
    root_dir.hash(&mut hasher);
    let hash = hasher.finish();
    cache_dir().join(format!("{:x}.json", hash))
}

pub fn load(root_dir: &str) -> Option<Vec<Fixture>> {
    let path = cache_path(root_dir);
    let data = std::fs::read_to_string(&path).ok()?;
    let fixtures: Vec<Fixture> = serde_json::from_str(&data).ok()?;
    eprintln!("pytest-fixtures-lsp: loaded {} fixtures from cache ({})", fixtures.len(), path.display());
    Some(fixtures)
}

pub fn save(root_dir: &str, fixtures: &[Fixture]) {
    let dir = cache_dir();
    let _ = std::fs::create_dir_all(&dir);
    let path = cache_path(root_dir);
    if let Ok(data) = serde_json::to_string(fixtures) {
        if std::fs::write(&path, data).is_ok() {
            eprintln!("pytest-fixtures-lsp: saved {} fixtures to cache ({})", fixtures.len(), path.display());
        }
    }
}
