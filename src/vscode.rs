//! Discovers what VS Code and its forks provide: languages, grammars, themes and icon themes

use crate::bundled;
use serde_json::{Map, Value, json};
use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};

pub struct GrammarEntry {
    /// VS Code language id, or the scope name for injection-only grammars.
    pub name: String,
    pub path: PathBuf,
}

pub struct ThemeEntry {
    pub label: String,
    pub path: PathBuf,
    pub dark: bool,
}

#[derive(Clone, Debug)]
pub struct IconThemeEntry {
    pub label: String,
    pub path: PathBuf,
}

#[derive(Default)]
pub struct Catalog {
    pub grammars: Vec<GrammarEntry>,
    pub themes: Vec<ThemeEntry>,
    pub icon_themes: Vec<IconThemeEntry>,
    /// Lower-case extension including the dot, e.g. ".rs" -> "rust".
    pub extensions: HashMap<String, String>,
    /// Lower-case exact filename, e.g. "makefile" -> "makefile".
    pub filenames: HashMap<String, String>,
    /// Comment delimiters per language id, from language-configuration.json.
    pub comments: HashMap<String, CommentSyntax>,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct CommentSyntax {
    pub line: Option<String>,
    pub block: Option<(String, String)>,
}

fn read_comment_syntax(path: &Path) -> Option<CommentSyntax> {
    let config = parse_jsonc(&fs::read_to_string(path).ok()?).ok()?;
    let comments = &config["comments"];
    // lineComment is a string, or an object with a `comment` field in newer VS Code.
    let line = comments["lineComment"]
        .as_str()
        .or(comments["lineComment"]["comment"].as_str())
        .map(str::to_owned);
    let block = match (comments["blockComment"][0].as_str(), comments["blockComment"][1].as_str()) {
        (Some(open), Some(close)) => Some((open.to_owned(), close.to_owned())),
        _ => None,
    };
    (line.is_some() || block.is_some()).then_some(CommentSyntax { line, block })
}

/// An editor's built-in extensions folder. Installs keep the app either directly in `resources/app` or in hash-named version folders; the newest version wins.
fn newest_install(root: &Path) -> Option<PathBuf> {
    let children = fs::read_dir(root).ok()?.flatten().map(|entry| entry.path());
    let mut best: Option<(Vec<u32>, PathBuf)> = None;
    for candidate in std::iter::once(root.to_path_buf()).chain(children) {
        let app = candidate.join("resources").join("app");
        let extensions = app.join("extensions");
        if !extensions.is_dir() {
            continue;
        }
        let Some(version) = fs::read_to_string(app.join("package.json"))
            .ok()
            .and_then(|text| parse_jsonc(&text).ok())
            .and_then(|v| v["version"].as_str().map(str::to_owned))
        else {
            continue;
        };
        let key: Vec<u32> = version.split('.').filter_map(|p| p.parse().ok()).collect();
        if best.as_ref().is_none_or(|(k, _)| key > *k) {
            best = Some((key, extensions));
        }
    }
    best.map(|(_, extensions)| extensions)
}

/// VS Code first, so its copy of anything shared with a fork is the one used.
fn vs_code_first(mut paths: Vec<PathBuf>, marker: &str) -> Vec<PathBuf> {
    paths.sort();
    paths.sort_by_key(|path| !path.to_string_lossy().contains(marker));
    paths
}

/// Every install of VS Code or a fork, found by shape rather than by name, so forks this app has never heard of work too.
fn editor_installs() -> Vec<PathBuf> {
    let local = std::env::var_os("LOCALAPPDATA").map(|p| PathBuf::from(p).join("Programs"));
    let program_files = std::env::var_os("ProgramFiles").map(PathBuf::from);
    let installs = [local, program_files]
        .into_iter()
        .flatten()
        .filter_map(|root| fs::read_dir(root).ok())
        .flat_map(|read| read.flatten())
        .filter_map(|entry| newest_install(&entry.path()))
        .collect();
    vs_code_first(installs, "Microsoft VS Code")
}

/// Where each editor keeps installed extensions: `~/.vscode/extensions`, `~/.kiro/extensions`…
fn user_extension_folders() -> Vec<PathBuf> {
    let Some(home) = std::env::var_os("USERPROFILE").map(PathBuf::from) else {
        return Vec::new();
    };
    let Ok(read) = fs::read_dir(home) else {
        return Vec::new();
    };
    let folders = read
        .flatten()
        .filter(|entry| entry.file_name().to_string_lossy().starts_with('.'))
        .map(|entry| entry.path().join("extensions"))
        .filter(|path| path.is_dir())
        .collect();
    vs_code_first(folders, ".vscode")
}

/// What the app can use: always the bundled set, plus anything editors provide.
pub fn scan(include_editors: bool) -> Catalog {
    let mut catalog = Catalog::default();
    if include_editors {
        for folder in editor_installs().into_iter().chain(user_extension_folders()) {
            scan_extensions(&folder, &mut catalog);
        }
    }
    for (id, extensions, filenames, (line, block)) in bundled::LANGUAGES {
        for extension in *extensions {
            catalog
                .extensions
                .entry((*extension).to_owned())
                .or_insert_with(|| (*id).to_owned());
        }
        for filename in *filenames {
            catalog
                .filenames
                .entry((*filename).to_owned())
                .or_insert_with(|| (*id).to_owned());
        }
        catalog.comments.entry((*id).to_owned()).or_insert_with(|| CommentSyntax {
            line: line.map(str::to_owned),
            block: block.map(|(open, close)| (open.to_owned(), close.to_owned())),
        });
    }
    catalog
}

fn scan_extensions(dir: &Path, catalog: &mut Catalog) {
    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let ext_dir = entry.path();
        let Ok(text) = fs::read_to_string(ext_dir.join("package.json")) else {
            continue;
        };
        let Ok(manifest) = parse_jsonc(&text) else {
            continue;
        };
        let contributes = &manifest["contributes"];

        for language in contributes["languages"].as_array().into_iter().flatten() {
            let Some(id) = language["id"].as_str() else {
                continue;
            };
            for ext in language["extensions"].as_array().into_iter().flatten() {
                if let Some(ext) = ext.as_str() {
                    catalog
                        .extensions
                        .entry(ext.to_lowercase())
                        .or_insert_with(|| id.to_owned());
                }
            }
            for name in language["filenames"].as_array().into_iter().flatten() {
                if let Some(name) = name.as_str() {
                    catalog
                        .filenames
                        .entry(name.to_lowercase())
                        .or_insert_with(|| id.to_owned());
                }
            }
            if !catalog.comments.contains_key(id) {
                if let Some(config) = language["configuration"].as_str() {
                    if let Some(syntax) = read_comment_syntax(&ext_dir.join(config)) {
                        catalog.comments.insert(id.to_owned(), syntax);
                    }
                }
            }
        }

        for grammar in contributes["grammars"].as_array().into_iter().flatten() {
            let (Some(path), Some(scope)) =
                (grammar["path"].as_str(), grammar["scopeName"].as_str())
            else {
                continue;
            };
            let name = grammar["language"].as_str().unwrap_or(scope).to_owned();
            if !catalog.grammars.iter().any(|g| g.name == name) {
                catalog.grammars.push(GrammarEntry {
                    name,
                    path: ext_dir.join(path),
                });
            }
        }

        for theme in contributes["themes"].as_array().into_iter().flatten() {
            let Some(path) = theme["path"].as_str() else {
                continue;
            };
            let label = theme["label"]
                .as_str()
                .or(theme["id"].as_str())
                .unwrap_or(path)
                .to_owned();
            let label = resolve_label(&label, &ext_dir).unwrap_or(label);
            // uiTheme is one of vs, vs-dark, hc-black, hc-light.
            let ui = theme["uiTheme"].as_str().unwrap_or("vs-dark");
            let dark = ui == "vs-dark" || ui == "hc-black";
            if !catalog.themes.iter().any(|t| t.label == label) {
                catalog.themes.push(ThemeEntry {
                    label,
                    path: ext_dir.join(path),
                    dark,
                });
            }
        }

        for theme in contributes["iconThemes"].as_array().into_iter().flatten() {
            let Some(path) = theme["path"].as_str() else {
                continue;
            };
            let label = theme["label"]
                .as_str()
                .or(theme["id"].as_str())
                .unwrap_or(path)
                .to_owned();
            let label = resolve_label(&label, &ext_dir).unwrap_or(label);
            let path = ext_dir.join(path);
            // Forks ship the same files under labels like "Seti (Kiro)".
            let duplicate = catalog
                .icon_themes
                .iter()
                .any(|t| t.label == label || t.path.file_name() == path.file_name());
            if !duplicate {
                catalog.icon_themes.push(IconThemeEntry { label, path });
            }
        }
    }
}

/// Strips `//` and `/* */` comments and trailing commas so strict JSON parsers accept VS Code's JSONC. Leaves string contents alone.
pub fn strip_jsonc(text: &str) -> String {
    let bytes = text.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        match bytes[i] {
            b'"' => {
                let start = i;
                i += 1;
                while i < bytes.len() && bytes[i] != b'"' {
                    i += if bytes[i] == b'\\' { 2 } else { 1 };
                }
                i = (i + 1).min(bytes.len());
                out.extend_from_slice(&bytes[start..i]);
            }
            b'/' if bytes.get(i + 1) == Some(&b'/') => {
                while i < bytes.len() && bytes[i] != b'\n' {
                    i += 1;
                }
            }
            b'/' if bytes.get(i + 1) == Some(&b'*') => {
                i += 2;
                while i + 1 < bytes.len() && !(bytes[i] == b'*' && bytes[i + 1] == b'/') {
                    i += 1;
                }
                i = (i + 2).min(bytes.len());
            }
            b',' => {
                let j = skip_trivia(bytes, i + 1);
                if !matches!(bytes.get(j), Some(b'}' | b']')) {
                    out.push(b',');
                }
                i += 1;
            }
            b => {
                out.push(b);
                i += 1;
            }
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// Index of the next byte that is neither whitespace nor inside a comment.
fn skip_trivia(bytes: &[u8], mut j: usize) -> usize {
    loop {
        while j < bytes.len() && bytes[j].is_ascii_whitespace() {
            j += 1;
        }
        match (bytes.get(j), bytes.get(j + 1)) {
            (Some(b'/'), Some(b'/')) => {
                while j < bytes.len() && bytes[j] != b'\n' {
                    j += 1;
                }
            }
            (Some(b'/'), Some(b'*')) => {
                j += 2;
                while j + 1 < bytes.len() && !(bytes[j] == b'*' && bytes[j + 1] == b'/') {
                    j += 1;
                }
                j = (j + 2).min(bytes.len());
            }
            _ => return j,
        }
    }
}

/// Resolves a `%key%` manifest label through the extension's `package.nls.json`.
fn resolve_label(label: &str, ext_dir: &Path) -> Option<String> {
    let key = label.strip_prefix('%')?.strip_suffix('%')?;
    let text = fs::read_to_string(ext_dir.join("package.nls.json")).ok()?;
    let nls = parse_jsonc(&text).ok()?;
    match &nls[key] {
        Value::String(s) => Some(s.clone()),
        other => other["message"].as_str().map(str::to_owned),
    }
}

pub fn parse_jsonc(text: &str) -> Result<Value, String> {
    serde_json::from_str(strip_jsonc(text.trim_start_matches('\u{feff}')).as_str())
        .map_err(|e| e.to_string())
}

/// Reads a grammar and renames it to the VS Code language id, because giallo looks grammars up by their internal `name`, which is not unique or stable.
pub fn grammar_bytes(entry: &GrammarEntry) -> Result<Vec<u8>, String> {
    let text = fs::read_to_string(&entry.path).map_err(|e| e.to_string())?;
    let mut value = parse_jsonc(&text)?;
    value["name"] = Value::String(entry.name.clone());
    neutralize_unparseable(&mut value);
    serde_json::to_vec(&value).map_err(|e| e.to_string())
}

/// Follows a theme's `include` chain and returns one flat theme file. `colors` is reduced to `editor.*` on purpose: giallo takes whichever of `foreground` / `editor.foreground` it meets first, and the bare `foreground` key is VS Code's workbench UI colour, not the code colour.
pub fn flatten_theme(entry: &ThemeEntry) -> Result<(Vec<u8>, ThemeColors), String> {
    let (colors, mut token_colors) = load_chain(&entry.path, 0)?;
    // Some themes carry empty colour strings, which VS Code ignores and giallo rejects.
    for rule in &mut token_colors {
        if let Some(settings) = rule["settings"].as_object_mut() {
            settings.retain(|_, value| value.as_str() != Some(""));
        }
    }
    let default_fg = if entry.dark { "#D4D4D4" } else { "#000000" };
    let default_bg = if entry.dark { "#1E1E1E" } else { "#FFFFFF" };
    let get = |key: &str, fallback: &str| {
        colors
            .get(key)
            .and_then(Value::as_str)
            .unwrap_or(fallback)
            .to_owned()
    };
    let palette = ThemeColors {
        id: entry.label.clone(),
        foreground: get("editor.foreground", default_fg),
        background: get("editor.background", default_bg),
        line_number: get("editorLineNumber.foreground", "#858585"),
    };
    let flat = json!({
        "name": entry.label,
        "type": if entry.dark { "dark" } else { "light" },
        "colors": {
            "editor.foreground": palette.foreground,
            "editor.background": palette.background,
        },
        "tokenColors": token_colors,
    });
    let bytes = serde_json::to_vec(&flat).map_err(|e| e.to_string())?;
    Ok((bytes, palette))
}

#[derive(Clone)]
pub struct ThemeColors {
    /// The name the highlighter knows this theme by.
    pub id: String,
    pub foreground: String,
    pub background: String,
    pub line_number: String,
}

fn load_chain(path: &Path, depth: u8) -> Result<(Map<String, Value>, Vec<Value>), String> {
    if depth > 8 {
        return Err("theme include chain too deep".into());
    }
    let text = fs::read_to_string(path).map_err(|e| format!("{}: {e}", path.display()))?;
    let theme = parse_jsonc(&text).map_err(|e| format!("{}: {e}", path.display()))?;

    let (mut colors, mut tokens) = match theme["include"].as_str() {
        Some(include) => load_chain(&path.parent().unwrap_or(path).join(include), depth + 1)?,
        None => Default::default(),
    };
    if let Some(own) = theme["colors"].as_object() {
        colors.extend(own.clone());
    }
    if let Some(own) = theme["tokenColors"].as_array() {
        tokens.extend(own.iter().cloned());
    }
    Ok((colors, tokens))
}

/// The options giallo compiles grammar regexes with.
pub fn regex_options() -> fancy_regex::RegexOptionsBuilder {
    let mut builder = fancy_regex::RegexOptionsBuilder::new();
    builder
        .oniguruma_mode(true)
        .multi_line(true)
        .build_delegate_prefilter(false)
        .allow_input_assertion_overrides(true);
    builder
}

const NEVER_MATCHES: &str = r"[^\s\S]";

/// Replaces patterns giallo would panic on while loading (e.g. LaTeX's `\V`) with one that never matches. Returns how many were replaced.
fn neutralize_unparseable(value: &mut Value) -> usize {
    match value {
        Value::Object(map) => {
            let mut replaced = 0;
            for key in ["match", "begin", "end", "while"] {
                if let Some(slot) = map.get_mut(key) {
                    let bad = slot
                        .as_str()
                        .is_some_and(|p| regex_options().required_bytes(p).is_err());
                    if bad {
                        *slot = Value::String(NEVER_MATCHES.into());
                        replaced += 1;
                    }
                }
            }
            replaced + map.values_mut().map(neutralize_unparseable).sum::<usize>()
        }
        Value::Array(items) => items.iter_mut().map(neutralize_unparseable).sum(),
        _ => 0,
    }
}
