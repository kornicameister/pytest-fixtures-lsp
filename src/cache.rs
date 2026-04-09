use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};
use std::path::PathBuf;

use crate::fixtures::Fixture;

fn hash_str(s: &str) -> String {
    let mut hasher = DefaultHasher::new();
    s.hash(&mut hasher);
    format!("{:x}", hasher.finish())
}

fn project_dir(root_dir: &str) -> PathBuf {
    let home = std::env::var("HOME").unwrap_or_else(|_| "/tmp".into());
    PathBuf::from(home)
        .join(".cache/pytest-fixtures-lsp")
        .join(hash_str(root_dir))
}

/// Load all cached fixtures for a project (global + all packages)
pub fn load_all(root_dir: &str) -> Vec<Fixture> {
    let dir = project_dir(root_dir);
    let Ok(entries) = std::fs::read_dir(&dir) else {
        return Vec::new();
    };

    let mut all = Vec::new();
    let mut seen = std::collections::HashSet::new();

    // Load global first for priority
    let global_path = dir.join("global.json");
    if let Some(fixtures) = load_file(&global_path) {
        for f in fixtures {
            if seen.insert(f.name.clone()) {
                all.push(f);
            }
        }
    }

    // Then packages
    for entry in entries.flatten() {
        let path = entry.path();
        if path.extension().map(|e| e == "json").unwrap_or(false)
            && path.file_stem().map(|s| s != "global").unwrap_or(true)
        {
            if let Some(fixtures) = load_file(&path) {
                for f in fixtures {
                    if seen.insert(f.name.clone()) {
                        all.push(f);
                    }
                }
            }
        }
    }

    if !all.is_empty() {
        eprintln!("pytest-fixtures-lsp: loaded {} fixtures from cache", all.len());
    }
    all
}

/// Save fixtures for a specific source (global or package)
pub fn save(root_dir: &str, source: &str, fixtures: &[Fixture]) {
    let dir = project_dir(root_dir);
    let _ = std::fs::create_dir_all(&dir);

    let filename = if source == "global" {
        "global.json".to_string()
    } else {
        format!("{}.json", hash_str(source))
    };

    let path = dir.join(filename);
    if let Ok(data) = serde_json::to_string(fixtures) {
        let _ = std::fs::write(&path, data);
        eprintln!("pytest-fixtures-lsp: cached {} fixtures for '{}'", fixtures.len(), source);
    }
}

fn load_file(path: &PathBuf) -> Option<Vec<Fixture>> {
    let data = std::fs::read_to_string(path).ok()?;
    serde_json::from_str(&data).ok()
}
