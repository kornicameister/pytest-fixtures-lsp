mod cache;
mod fixtures;
mod runner;

use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::RwLock;
use tower_lsp_server::jsonrpc::Result;
use tower_lsp_server::ls_types::*;
use tower_lsp_server::{Client, LanguageServer, LspService, Server};

use fixtures::Fixture;

fn uri_to_path(uri: &Uri) -> String {
    uri.as_str().strip_prefix("file://").unwrap_or(uri.as_str()).to_string()
}

#[derive(Debug)]
struct Backend {
    client: Client,
    fixtures: Arc<RwLock<Vec<Fixture>>>,
    root_dir: Arc<RwLock<Option<String>>>,
    documents: Arc<RwLock<HashMap<String, String>>>,
}

impl Backend {
    /// Determine which package a file belongs to based on root_dir
    async fn file_package(&self, file_path: &str) -> Option<String> {
        let root = self.root_dir.read().await;
        let root = root.as_deref()?;
        let relative = file_path.strip_prefix(root)?.trim_start_matches('/');

        let rel_path = std::path::Path::new(relative);
        let mut dir = rel_path.parent();
        while let Some(d) = dir {
            let candidate = std::path::Path::new(root).join(d).join("pyproject.toml");
            if candidate.exists() && d != std::path::Path::new("") {
                return Some(d.to_string_lossy().to_string());
            }
            dir = d.parent();
        }
        None
    }

    fn word_at(line: &str, col: usize) -> &str {
        let bytes = line.as_bytes();
        let start = (0..col)
            .rev()
            .take_while(|&i| i < bytes.len() && (bytes[i].is_ascii_alphanumeric() || bytes[i] == b'_'))
            .last()
            .unwrap_or(col);
        let end = (col..bytes.len())
            .take_while(|&i| bytes[i].is_ascii_alphanumeric() || bytes[i] == b'_')
            .last()
            .map(|i| i + 1)
            .unwrap_or(col);
        &line[start..end]
    }
}

impl LanguageServer for Backend {
    async fn initialize(&self, params: InitializeParams) -> Result<InitializeResult> {
        #[allow(deprecated)]
        let root = params.root_uri.as_ref().map(|uri| uri_to_path(uri));

        if let Some(ref dir) = root {
            *self.root_dir.write().await = root.clone();
        }

        Ok(InitializeResult {
            capabilities: ServerCapabilities {
                text_document_sync: Some(TextDocumentSyncCapability::Kind(
                    TextDocumentSyncKind::FULL,
                )),
                completion_provider: Some(CompletionOptions {
                    trigger_characters: Some(vec!["(".to_string(), ",".to_string(), " ".to_string()]),
                    ..Default::default()
                }),
                hover_provider: Some(HoverProviderCapability::Simple(true)),
                ..Default::default()
            },
            ..Default::default()
        })
    }

    async fn initialized(&self, _params: InitializedParams) {
        let root_dir = self.root_dir.read().await.clone();
        let fixtures = self.fixtures.clone();
        let client = self.client.clone();

        tokio::spawn(async move {
            if let Some(ref dir) = root_dir {
                // 1. Load from cache instantly
                let cached = cache::load_all(dir);
                if !cached.is_empty() {
                    let count = cached.len();
                    *fixtures.write().await = cached;
                    client
                        .log_message(MessageType::INFO, format!("pytest-fixtures-lsp: {} fixtures (cached)", count))
                        .await;
                }

                // 2. Refresh incrementally from pytest
                fixtures::collect_all(dir, &fixtures).await;
                let count = fixtures.read().await.len();
                client
                    .log_message(MessageType::INFO, format!("pytest-fixtures-lsp: {} fixtures (refreshed)", count))
                    .await;
            }
        });
    }

    async fn shutdown(&self) -> Result<()> {
        Ok(())
    }

    async fn did_open(&self, params: DidOpenTextDocumentParams) {
        let key = uri_to_path(&params.text_document.uri);
        self.documents.write().await.insert(key, params.text_document.text);
    }

    async fn did_change(&self, params: DidChangeTextDocumentParams) {
        if let Some(change) = params.content_changes.into_iter().last() {
            let key = uri_to_path(&params.text_document.uri);
            self.documents.write().await.insert(key, change.text);
        }
    }

    async fn did_close(&self, params: DidCloseTextDocumentParams) {
        let key = uri_to_path(&params.text_document.uri);
        self.documents.write().await.remove(&key);
    }

    async fn did_save(&self, params: DidSaveTextDocumentParams) {
        let path = uri_to_path(&params.text_document.uri);
        if path.ends_with("conftest.py") {
            let root_dir = self.root_dir.read().await.clone();
            let fixtures = self.fixtures.clone();
            let client = self.client.clone();

            tokio::spawn(async move {
                if let Some(ref dir) = root_dir {
                    fixtures::collect_all(dir, &fixtures).await;
                    let count = fixtures.read().await.len();
                    client
                        .log_message(MessageType::INFO, format!("pytest-fixtures-lsp: {} fixtures (refreshed)", count))
                        .await;
                }
            });
        }
    }

    async fn completion(&self, params: CompletionParams) -> Result<Option<CompletionResponse>> {
        let uri = &params.text_document_position.text_document.uri;
        let path = uri_to_path(uri);

        let filename = path.rsplit('/').next().unwrap_or("");
        if !filename.starts_with("test_") && !filename.ends_with("_test.py") {
            eprintln!("pytest-fixtures-lsp: skip completion, not a test file: {}", filename);
            return Ok(None);
        }

        let fixtures = self.fixtures.read().await;
        eprintln!("pytest-fixtures-lsp: completion requested, {} fixtures available", fixtures.len());

        // Filter: show global + fixtures from the same package
        let file_pkg = self.file_package(&path).await;

        let items: Vec<CompletionItem> = fixtures
            .iter()
            .filter(|f| f.source == "global" || Some(&f.source) == file_pkg.as_ref())
            .map(|f| {
                let detail = format!("pytest [{}][{}]", f.scope, if f.source == "global" { "global" } else { &f.source });
                CompletionItem {
                    label: f.name.clone(),
                    label_details: Some(CompletionItemLabelDetails {
                        detail: f.return_type.as_ref().map(|t| format!(" → {}", t)),
                        description: Some(detail.clone()),
                    }),
                    kind: Some(CompletionItemKind::INTERFACE),
                    detail: Some(detail),
                    insert_text: Some(format!("{}: ${{1:{}}}", f.name, f.return_type.as_deref().unwrap_or("Any"))),
                    insert_text_format: Some(InsertTextFormat::SNIPPET),
                    sort_text: Some(format!("!0{}", f.name)),
                    documentation: if f.docstring.is_empty() {
                        None
                    } else {
                        Some(Documentation::MarkupContent(MarkupContent {
                            kind: MarkupKind::Markdown,
                            value: format!("{}\n\n*{}*", f.docstring, f.location),
                        }))
                    },
                    ..Default::default()
                }
            })
            .collect();

        Ok(Some(CompletionResponse::Array(items)))
    }

    async fn hover(&self, params: HoverParams) -> Result<Option<Hover>> {
        let uri = &params.text_document_position_params.text_document.uri;
        let pos = &params.text_document_position_params.position;
        let path = uri_to_path(uri);

        let docs = self.documents.read().await;
        let Some(text) = docs.get(&path) else {
            return Ok(None);
        };

        let Some(line) = text.lines().nth(pos.line as usize) else {
            return Ok(None);
        };

        let word = Self::word_at(line, pos.character as usize);
        if word.is_empty() {
            return Ok(None);
        }

        let fixtures = self.fixtures.read().await;
        let Some(fixture) = fixtures.iter().find(|f| f.name == word) else {
            return Ok(None);
        };

        let mut content = format!("**{}** `[{}]`", fixture.name, fixture.scope);
        if fixture.source != "global" {
            content.push_str(&format!(" 📦 `{}`", fixture.source));
        }
        content.push_str("\n\n");
        if let Some(ref rt) = fixture.return_type {
            content.push_str(&format!("Returns: `{}`\n\n", rt));
        }
        if !fixture.docstring.is_empty() {
            content.push_str(&fixture.docstring);
            content.push_str("\n\n");
        }
        content.push_str(&format!("*{}*", fixture.location));

        Ok(Some(Hover {
            contents: HoverContents::Markup(MarkupContent {
                kind: MarkupKind::Markdown,
                value: content,
            }),
            range: None,
        }))
    }
}

#[tokio::main]
async fn main() {
    let stdin = tokio::io::stdin();
    let stdout = tokio::io::stdout();

    let (service, socket) = LspService::new(|client| Backend {
        client,
        fixtures: Arc::new(RwLock::new(Vec::new())),
        root_dir: Arc::new(RwLock::new(None)),
        documents: Arc::new(RwLock::new(HashMap::new())),
    });

    Server::new(stdin, stdout, socket).serve(service).await;
}
