use std::path::Path;

/// Strategy for running pytest in different project types.
pub trait PytestRunner: Send + Sync {
    fn name(&self) -> &'static str;
    fn detect(&self, root_dir: &Path) -> bool;
    fn command(&self, root_dir: &Path) -> (String, Vec<String>);
}

pub struct UvRunner;
pub struct PoetryRunner;
pub struct PipenvRunner;
pub struct VenvRunner;
pub struct SystemRunner;

impl PytestRunner for UvRunner {
    fn name(&self) -> &'static str { "uv" }
    fn detect(&self, root_dir: &Path) -> bool {
        root_dir.join("uv.lock").exists()
    }
    fn command(&self, _root_dir: &Path) -> (String, Vec<String>) {
        ("uv".into(), vec!["run".into(), "pytest".into()])
    }
}

impl PytestRunner for PoetryRunner {
    fn name(&self) -> &'static str { "poetry" }
    fn detect(&self, root_dir: &Path) -> bool {
        root_dir.join("poetry.lock").exists()
    }
    fn command(&self, _root_dir: &Path) -> (String, Vec<String>) {
        ("poetry".into(), vec!["run".into(), "pytest".into()])
    }
}

impl PytestRunner for PipenvRunner {
    fn name(&self) -> &'static str { "pipenv" }
    fn detect(&self, root_dir: &Path) -> bool {
        root_dir.join("Pipfile.lock").exists()
    }
    fn command(&self, _root_dir: &Path) -> (String, Vec<String>) {
        ("pipenv".into(), vec!["run".into(), "pytest".into()])
    }
}

impl PytestRunner for VenvRunner {
    fn name(&self) -> &'static str { "venv" }
    fn detect(&self, root_dir: &Path) -> bool {
        root_dir.join(".venv/bin/pytest").exists() || root_dir.join("venv/bin/pytest").exists()
    }
    fn command(&self, root_dir: &Path) -> (String, Vec<String>) {
        let pytest = if root_dir.join(".venv/bin/pytest").exists() {
            root_dir.join(".venv/bin/pytest")
        } else {
            root_dir.join("venv/bin/pytest")
        };
        (pytest.to_string_lossy().into(), vec![])
    }
}

impl PytestRunner for SystemRunner {
    fn name(&self) -> &'static str { "system" }
    fn detect(&self, _root_dir: &Path) -> bool { true }
    fn command(&self, _root_dir: &Path) -> (String, Vec<String>) {
        ("pytest".into(), vec![])
    }
}

/// Detect project type and return the appropriate runner.
/// Priority: uv > poetry > pipenv > venv > system
pub fn detect(root_dir: &Path) -> &'static dyn PytestRunner {
    static RUNNERS: &[&dyn PytestRunner] = &[
        &UvRunner,
        &PoetryRunner,
        &PipenvRunner,
        &VenvRunner,
        &SystemRunner,
    ];
    RUNNERS.iter().find(|r| r.detect(root_dir)).copied().unwrap()
}
