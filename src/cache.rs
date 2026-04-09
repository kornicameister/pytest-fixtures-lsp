use std::path::Path;

use crate::fixtures::Fixture;

const CACHE_FILE: &str = ".pytest-fixtures-lsp.json";

pub fn load(root_dir: &str) -> Option<Vec<Fixture>> {
    let path = Path::new(root_dir).join(CACHE_FILE);
    let data = std::fs::read_to_string(&path).ok()?;
    let fixtures: Vec<Fixture> = serde_json::from_str(&data).ok()?;
    eprintln!("pytest-fixtures-lsp: loaded {} fixtures from cache", fixtures.len());
    Some(fixtures)
}

pub fn save(root_dir: &str, fixtures: &[Fixture]) {
    let path = Path::new(root_dir).join(CACHE_FILE);
    if let Ok(data) = serde_json::to_string_pretty(fixtures) {
        if std::fs::write(&path, data).is_ok() {
            eprintln!("pytest-fixtures-lsp: saved {} fixtures to cache", fixtures.len());
        }
    }
}
