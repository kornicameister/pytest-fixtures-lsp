use serde::Deserialize;

#[derive(Debug, Clone, Deserialize)]
pub struct Fixture {
    pub name: String,
    pub scope: String,
    pub docstring: String,
    pub return_type: Option<String>,
    pub location: String,
}

/// Run `pytest --fixtures -q` and parse output into Vec<Fixture>
pub async fn collect(root_dir: &str) -> Vec<Fixture> {
    let output = tokio::process::Command::new("pytest")
        .args(["--fixtures", "-q"])
        .current_dir(root_dir)
        .output()
        .await;

    let output = match output {
        Ok(o) => o,
        Err(_) => return Vec::new(),
    };

    let stdout = String::from_utf8_lossy(&output.stdout);
    parse_fixtures_output(&stdout)
}

fn parse_fixtures_output(output: &str) -> Vec<Fixture> {
    let mut fixtures = Vec::new();
    let mut current_name: Option<String> = None;
    let mut current_scope = String::from("function");
    let mut current_location = String::new();
    let mut current_doc_lines: Vec<String> = Vec::new();

    for line in output.lines() {
        if line.starts_with("=") || line.starts_with("-") || line.trim().is_empty() {
            // flush current fixture
            if let Some(name) = current_name.take() {
                let docstring = current_doc_lines.join("\n").trim().to_string();
                let return_type = extract_return_type(&docstring);
                fixtures.push(Fixture {
                    name,
                    scope: current_scope.clone(),
                    docstring,
                    return_type,
                    location: current_location.clone(),
                });
                current_doc_lines.clear();
                current_scope = String::from("function");
                current_location.clear();
            }
            continue;
        }

        // fixture line: "name -- description" or "name [scope] -- description"
        if !line.starts_with(' ') && !line.starts_with('\t') {
            // flush previous
            if let Some(name) = current_name.take() {
                let docstring = current_doc_lines.join("\n").trim().to_string();
                let return_type = extract_return_type(&docstring);
                fixtures.push(Fixture {
                    name,
                    scope: current_scope.clone(),
                    docstring,
                    return_type,
                    location: current_location.clone(),
                });
                current_doc_lines.clear();
                current_scope = String::from("function");
                current_location.clear();
            }

            let line = line.trim();
            // parse "name [scope] -- location"
            let (name_part, rest) = match line.split_once(" -- ") {
                Some((n, r)) => (n.trim(), r.trim().to_string()),
                None => (line, String::new()),
            };

            // extract scope if present: "name [scope]"
            if let Some(bracket_start) = name_part.find('[') {
                current_name = Some(name_part[..bracket_start].trim().to_string());
                if let Some(bracket_end) = name_part.find(']') {
                    current_scope = name_part[bracket_start + 1..bracket_end].to_string();
                }
            } else {
                current_name = Some(name_part.to_string());
            }
            current_location = rest;
        } else if current_name.is_some() {
            // docstring continuation line
            current_doc_lines.push(line.trim().to_string());
        }
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
