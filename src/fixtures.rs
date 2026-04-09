use serde::Deserialize;
use std::collections::HashMap;
use std::path::Path;
use std::sync::Arc;

use crate::runner;

#[derive(Debug, Clone, Deserialize, serde::Serialize)]
pub struct Fixture {
    pub name: String,
    pub scope: String,
    pub docstring: String,
    pub return_type: Option<String>,
    pub location: String,
    /// "global" or package name like "mcp/harbor"
    pub source: String,
}

/// Collect fixtures from root + all sub-packages, updating shared state incrementally
pub async fn collect_all(
    root_dir: &str,
    fixtures: &Arc<tokio::sync::RwLock<Vec<Fixture>>>,
) {
    let root = Path::new(root_dir);
    let strategy = runner::detect(root);

    // 1. Global fixtures
    let global = run_pytest(root_dir, strategy, "global").await;
    eprintln!("pytest-fixtures-lsp: global fixtures: {}", global.len());
    {
        *fixtures.write().await = global;
        let guard = fixtures.read().await;
        crate::cache::save(root_dir, &guard);
    }

    // 2. Sub-packages
    let packages = find_packages(root);
    eprintln!("pytest-fixtures-lsp: found {} sub-packages", packages.len());

    for pkg_path in &packages {
        let pkg_name = pkg_path.strip_prefix(root).unwrap_or(pkg_path);
        let label = pkg_name.to_string_lossy().to_string();
        eprintln!("pytest-fixtures-lsp: scanning package: {}", label);

        let pkg_fixtures = run_pytest(&pkg_path.to_string_lossy(), strategy, &label).await;
        eprintln!("pytest-fixtures-lsp:   {} fixtures from {}", pkg_fixtures.len(), label);

        if !pkg_fixtures.is_empty() {
            let mut all = fixtures.write().await;
            let existing: std::collections::HashSet<String> = all.iter().map(|f| f.name.clone()).collect();
            for f in pkg_fixtures {
                if !existing.contains(&f.name) {
                    all.push(f);
                }
            }
            crate::cache::save(root_dir, &all);
            drop(all);
        }
    }

    eprintln!("pytest-fixtures-lsp: total fixtures: {}", fixtures.read().await.len());
}

/// Find sub-directories that have pyproject.toml AND (conftest.py or tests/)
fn find_packages(root: &Path) -> Vec<std::path::PathBuf> {
    let mut packages = Vec::new();
    scan_for_packages(root, root, &mut packages, 0);
    packages
}

fn scan_for_packages(dir: &Path, root: &Path, packages: &mut Vec<std::path::PathBuf>, depth: usize) {
    if depth > 4 { return; }

    let Ok(entries) = std::fs::read_dir(dir) else { return };

    for entry in entries.flatten() {
        let path = entry.path();
        if !path.is_dir() { continue; }

        let name = path.file_name().unwrap_or_default().to_string_lossy();
        // skip hidden dirs, venvs, cdk output, node_modules
        if name.starts_with('.') || name == "node_modules" || name == "__pycache__"
            || name == "cdk.out" || name == "venv" || name == ".venv" { continue; }

        // is this a sub-package? (has pyproject.toml and is not root)
        if path != root && path.join("pyproject.toml").exists() {
            let has_tests = path.join("conftest.py").exists()
                || path.join("tests").is_dir()
                || path.join("tests/conftest.py").exists();
            if has_tests {
                packages.push(path.clone());
            }
        }

        scan_for_packages(&path, root, packages, depth + 1);
    }
}

async fn run_pytest(dir: &str, strategy: &dyn runner::PytestRunner, source: &str) -> Vec<Fixture> {
    let root = Path::new(dir);
    let (cmd, base_args) = strategy.command(root);

    let mut args = base_args;
    args.extend(["--fixtures".into(), "-q".into()]);

    let output = tokio::process::Command::new(&cmd)
        .args(&args)
        .current_dir(dir)
        .output()
        .await;

    let output = match output {
        Ok(o) if o.status.success() => o,
        Ok(o) => {
            eprintln!("pytest-fixtures-lsp: pytest failed in {}: exit {}", dir, o.status);
            return Vec::new();
        }
        Err(e) => {
            eprintln!("pytest-fixtures-lsp: failed to run pytest in {}: {}", dir, e);
            return Vec::new();
        }
    };

    let stdout = String::from_utf8_lossy(&output.stdout);
    parse_fixtures_output(&stdout, source)
}

fn parse_fixtures_output(output: &str, source: &str) -> Vec<Fixture> {
    let mut fixtures = Vec::new();
    let mut current_name: Option<String> = None;
    let mut current_scope = String::from("function");
    let mut current_location = String::new();
    let mut current_doc_lines: Vec<String> = Vec::new();

    let is_fixture_line = |line: &str| -> bool {
        let t = line.trim();
        !t.is_empty() && t.contains(" -- ")
            && t.chars().next().map(|c| c.is_ascii_alphabetic() || c == '_').unwrap_or(false)
    };

    let is_docstring_line = |line: &str| -> bool {
        line.starts_with("    ") || line.starts_with('\t')
    };

    for line in output.lines() {
        if is_fixture_line(line) {
            if let Some(name) = current_name.take() {
                let docstring = current_doc_lines.join("\n").trim().to_string();
                let return_type = extract_return_type(&docstring);
                fixtures.push(Fixture { name, scope: current_scope.clone(), docstring, return_type, location: current_location.clone(), source: source.to_string() });
                current_doc_lines.clear();
                current_scope = String::from("function");
                current_location.clear();
            }

            let trimmed = line.trim();
            let (name_part, rest) = match trimmed.split_once(" -- ") {
                Some((n, r)) => (n.trim(), r.trim().to_string()),
                None => continue,
            };

            if let Some(bracket_start) = name_part.find('[') {
                current_name = Some(name_part[..bracket_start].trim().to_string());
                if let Some(bracket_end) = name_part.find(']') {
                    current_scope = name_part[bracket_start + 1..bracket_end].to_string();
                }
            } else {
                current_name = Some(name_part.to_string());
            }
            current_location = rest;
        } else if is_docstring_line(line) && current_name.is_some() {
            current_doc_lines.push(line.trim().to_string());
        }
    }

    if let Some(name) = current_name.take() {
        let docstring = current_doc_lines.join("\n").trim().to_string();
        let return_type = extract_return_type(&docstring);
        fixtures.push(Fixture { name, scope: current_scope, docstring, return_type, location: current_location, source: source.to_string() });
    }

    fixtures
}

fn extract_return_type(docstring: &str) -> Option<String> {
    for line in docstring.lines() {
        let trimmed = line.trim().to_lowercase();
        if trimmed.starts_with("returns") || trimmed.starts_with("return type") {
            if let Some((_prefix, type_part)) = line.split_once(':') {
                let t = type_part.trim().to_string();
                if !t.is_empty() {
                    return Some(t);
                }
            }
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_basic() {
        let output = r#"
tmp_path [function] -- /usr/lib/python3.12/site-packages/_pytest/tmpdir.py
    Return a temporary directory path object.

capsys [function] -- /usr/lib/python3.12/site-packages/_pytest/capture.py
    Enable text capturing of writes to sys.stdout and sys.stderr.

my_fixture -- conftest.py
    Custom project fixture.
"#;
        let fixtures = parse_fixtures_output(output, "global");
        assert_eq!(fixtures.len(), 3);
        assert_eq!(fixtures[0].name, "tmp_path");
        assert_eq!(fixtures[0].source, "global");
        assert_eq!(fixtures[2].name, "my_fixture");
        assert_eq!(fixtures[2].source, "global");
    }
}
