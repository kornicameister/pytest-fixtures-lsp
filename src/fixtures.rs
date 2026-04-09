use serde::Deserialize;

#[derive(Debug, Clone, Deserialize)]
pub struct Fixture {
    pub name: String,
    pub scope: String,
    pub docstring: String,
    pub return_type: Option<String>,
    pub location: String,
}

use crate::runner;

/// Run `pytest --fixtures -q` and parse output into Vec<Fixture>
pub async fn collect(root_dir: &str) -> Vec<Fixture> {
    let root = std::path::Path::new(root_dir);
    let strategy = runner::detect(root);
    let (cmd, base_args) = strategy.command(root);

    let mut args = base_args;
    args.extend(["--fixtures".into(), "-q".into()]);

    eprintln!("pytest-fixtures-lsp: strategy={}, cmd={} {}", strategy.name(), cmd, args.join(" "));

    let output = tokio::process::Command::new(&cmd)
        .args(&args)
        .current_dir(root_dir)
        .output()
        .await;

    let output = match output {
        Ok(o) if o.status.success() => o,
        Ok(o) => {
            eprintln!("pytest-fixtures-lsp: pytest failed (exit {}): {}", o.status, String::from_utf8_lossy(&o.stderr));
            return Vec::new();
        }
        Err(e) => {
            eprintln!("pytest-fixtures-lsp: failed to run pytest: {}", e);
            return Vec::new();
        }
    };

    let stdout = String::from_utf8_lossy(&output.stdout);
    let fixtures = parse_fixtures_output(&stdout);
    eprintln!("pytest-fixtures-lsp: parsed {} fixtures", fixtures.len());
    for f in &fixtures {
        eprintln!("  fixture: {} [{}] type={:?} loc={}", f.name, f.scope, f.return_type, f.location);
    }
    fixtures
}

fn parse_fixtures_output(output: &str) -> Vec<Fixture> {
    let mut fixtures = Vec::new();
    let mut current_name: Option<String> = None;
    let mut current_scope = String::from("function");
    let mut current_location = String::new();
    let mut current_doc_lines: Vec<String> = Vec::new();

    let is_fixture_line = |line: &str| -> bool {
        let t = line.trim();
        // fixture lines: "name -- path" or "name [scope] -- path"
        !t.is_empty() && !t.starts_with(' ') && t.contains(" -- ")
            && t.chars().next().map(|c| c.is_ascii_alphabetic() || c == '_').unwrap_or(false)
    };

    let is_docstring_line = |line: &str| -> bool {
        line.starts_with("    ") || line.starts_with('\t')
    };

    for line in output.lines() {
        if is_fixture_line(line) {
            // flush previous
            if let Some(name) = current_name.take() {
                let docstring = current_doc_lines.join("\n").trim().to_string();
                let return_type = extract_return_type(&docstring);
                fixtures.push(Fixture { name, scope: current_scope.clone(), docstring, return_type, location: current_location.clone() });
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
        // skip everything else (errors, separators, etc.)
    }

    // flush last
    if let Some(name) = current_name.take() {
        let docstring = current_doc_lines.join("\n").trim().to_string();
        let return_type = extract_return_type(&docstring);
        fixtures.push(Fixture {
            name,
            scope: current_scope,
            docstring,
            return_type,
            location: current_location,
        });
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
        let fixtures = parse_fixtures_output(output);
        assert_eq!(fixtures.len(), 3);
        assert_eq!(fixtures[0].name, "tmp_path");
        assert_eq!(fixtures[0].scope, "function");
        assert!(fixtures[0].docstring.contains("temporary directory"));
        assert_eq!(fixtures[2].name, "my_fixture");
        assert_eq!(fixtures[2].location, "conftest.py");
    }
}
