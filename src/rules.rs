//! What "Fix" does to a comment, decided from settings the user controls. Fixed order: protections first, then deletion, then squashing

use crate::cleanup::{parse, squash};
use crate::highlight::{Hunk, HunkKind};
use crate::vscode::CommentSyntax;
use fancy_regex::Regex;
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::LazyLock;

const WORKSPACE_FILE: &str = ".commentcleanup.json";

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    pub protect_directives: bool,
    pub protect_licenses: bool,
    pub protect_todos: bool,
    pub protect_patterns: Vec<String>,

    pub squash_enabled: bool,
    pub squash_max_width: u32,
    pub squash_max_lines: u32,
    pub squash_doc_comments: bool,
    pub squash_lists: bool,

    pub delete_empty: bool,
    pub delete_dividers: bool,
    pub delete_commented_code: bool,
    pub delete_patterns: Vec<String>,

    pub theme: Option<String>,

    pub icon_theme: Option<String>,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            protect_directives: true,
            protect_licenses: true,
            protect_todos: true,
            protect_patterns: Vec::new(),
            squash_enabled: true,
            squash_max_width: 120,
            squash_max_lines: 4,
            squash_doc_comments: false,
            squash_lists: false,
            delete_empty: true,
            delete_dividers: true,
            delete_commented_code: false,
            delete_patterns: Vec::new(),
            theme: None,
            icon_theme: None,
        }
    }
}

fn config_dir() -> Option<PathBuf> {
    std::env::var_os("COMMENTCLEANUP_CONFIG_DIR")
        .map(PathBuf::from)
        .or_else(|| Some(PathBuf::from(std::env::var_os("APPDATA")?).join("CommentCleanup")))
}

pub fn config_file(name: &str) -> Option<PathBuf> {
    Some(config_dir()?.join(name))
}

pub fn app_settings_path() -> Option<PathBuf> {
    config_file("settings.json")
}

pub fn workspace_settings_path(root: &Path) -> PathBuf {
    root.join(WORKSPACE_FILE)
}

pub fn read_settings(path: &Path) -> Option<Settings> {
    serde_json::from_str(&fs::read_to_string(path).ok()?).ok()
}

pub fn save_theme(name: &str) -> Result<(), String> {
    update_app_settings(|settings| settings.theme = Some(name.to_owned()))
}

pub fn save_icon_theme(name: &str) -> Result<(), String> {
    update_app_settings(|settings| settings.icon_theme = Some(name.to_owned()))
}

fn update_app_settings(change: impl FnOnce(&mut Settings)) -> Result<(), String> {
    let path = app_settings_path().ok_or("no settings folder")?;
    let mut settings = read_settings(&path).unwrap_or_default();
    change(&mut settings);
    write_settings(&path, &settings)
}

pub fn write_settings(path: &Path, settings: &Settings) -> Result<(), String> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    }
    let text = serde_json::to_string_pretty(settings).map_err(|e| e.to_string())?;
    fs::write(path, text).map_err(|e| format!("Couldn't save settings: {e}"))
}

#[derive(Clone, Debug, PartialEq)]
pub enum Decision {
    Keep,
    Protected(&'static str),
    Squash(String),
    Delete(&'static str),
}

impl Decision {
    pub fn is_protected(&self) -> bool {
        matches!(self, Decision::Protected(_))
    }

    pub fn is_fix(&self) -> bool {
        matches!(self, Decision::Squash(_) | Decision::Delete(_))
    }

    pub fn summary(&self) -> String {
        match self {
            Decision::Keep => "Left alone".to_owned(),
            Decision::Protected(why) => format!("Protected: {why}"),
            Decision::Squash(_) => "Squashed".to_owned(),
            Decision::Delete(why) => format!("Deleted: {why}"),
        }
    }
}

pub fn try_comment(rules: &Rules, comment: &str, syntax: Option<&CommentSyntax>) -> Option<Decision> {
    let syntax = syntax?;
    let lines: Vec<&str> = comment.trim().lines().map(str::trim).collect();
    let raw = lines.join("\n");
    let line_run = syntax
        .line
        .as_deref()
        .is_some_and(|delimiter| !lines.is_empty() && lines.iter().all(|line| line.starts_with(delimiter)));
    let block = syntax
        .block
        .as_ref()
        .is_some_and(|(open, close)| raw.starts_with(open.as_str()) && raw.ends_with(close.as_str()));
    let kind = if line_run {
        HunkKind::LineRun
    } else if block {
        HunkKind::Block
    } else {
        return None;
    };
    let hunk = Hunk {
        first: 1,
        last: lines.len(),
        start: 0,
        end: lines.last()?.len(),
        kind,
        whole_line: true,
        raw,
        key: 0,
    };
    Some(rules.decide(&hunk, Some(syntax)))
}

fn pattern(source: &str) -> Regex {
    Regex::new(source).expect("built-in pattern")
}

static DIRECTIVE: LazyLock<Regex> = LazyLock::new(|| {
    pattern(
        r"(?ix)^(
            eslint | tslint | stylelint | biome-ignore | prettier-ignore | oxlint
          | @ts- | @flow | @jsx | @formatter | @license | @preserve
          | (istanbul|c8|v8|vitest)\s+ignore | webpack[A-Z]
          | noqa | flake8: | pylint: | mypy: | pyright: | ruff: | isort: | fmt: | type:\s*ignore
          | rubocop: | nolint | nosec | nosonar | noinspection | shellcheck | cspell | spell-checker
          | go: | \+build | \#?(end)?region\b | clang-format | clang-tidy | NOLINT
          | pragma | vim: | -\*- | sourceMappingURL | sourceURL | @ts-
        )",
    )
});
static LICENSE: LazyLock<Regex> = LazyLock::new(|| {
    pattern(r"(?i)\b(copyright|licen[cs]ed?\b|spdx-license|all rights reserved)")
});
static TODO: LazyLock<Regex> =
    LazyLock::new(|| pattern(r"(?i)^(todo|fixme|hack|xxx|bug)\b"));
static DIVIDER: LazyLock<Regex> = LazyLock::new(|| pattern(r"^([^\w\s])\1{2,}$"));
static LIST_ITEM: LazyLock<Regex> = LazyLock::new(|| pattern(r"^([-+*•]\s|\d+[.)]\s)"));

/// Deliberately strict, and every line of a comment has to match.
static CODE_LINE: LazyLock<Regex> = LazyLock::new(|| {
    pattern(
        r#"(?x)(
            [;{}]$
          | ^[\w.\[\]]+\s*(=|\+=|-=)\s*\S
          | ^[\w.]+\(.*\)$
          | ^(import|const|let|var|def|fn|function|\#include)\s+\S+\s*(=|\(|;|\{|$)
          | ^from\s+\S+\s+import\s
          | ^return\s+\S+;$
        )"#,
    )
});

fn is_match(regex: &Regex, text: &str) -> bool {
    regex.is_match(text).unwrap_or(false)
}

/// Settings compiled for matching, plus any custom patterns that didn't compile.
#[derive(Debug)]
pub struct Rules {
    settings: Settings,
    protect: Vec<Regex>,
    delete: Vec<Regex>,
    pub errors: Vec<String>,
}

impl Rules {
    pub fn new(settings: &Settings) -> Self {
        let mut errors = Vec::new();
        let mut compile = |sources: &[String]| -> Vec<Regex> {
            sources
                .iter()
                .filter_map(|source| match Regex::new(source) {
                    Ok(regex) => Some(regex),
                    Err(_) => {
                        errors.push(source.clone());
                        None
                    }
                })
                .collect()
        };
        let protect = compile(&settings.protect_patterns);
        let delete = compile(&settings.delete_patterns);
        Self {
            settings: settings.clone(),
            protect,
            delete,
            errors,
        }
    }

    pub fn decide(&self, hunk: &Hunk, syntax: Option<&CommentSyntax>) -> Decision {
        let s = &self.settings;
        let Some(parsed) = parse(hunk, syntax) else {
            return Decision::Keep;
        };
        let lines: Vec<&str> = parsed.lines.iter().copied().filter(|l| !l.is_empty()).collect();
        let any = |regex: &Regex| lines.iter().any(|l| is_match(regex, l));
        let all = |regex: &Regex| !lines.is_empty() && lines.iter().all(|l| is_match(regex, l));
        let custom = |regexes: &[Regex]| regexes.iter().any(|r| any(r) || is_match(r, &hunk.raw));

        let shebang = hunk.first == 0 && hunk.raw.starts_with("#!");
        if s.protect_directives && (shebang || any(&DIRECTIVE)) {
            return Decision::Protected("tool directive");
        }
        if s.protect_licenses && any(&LICENSE) {
            return Decision::Protected("licence notice");
        }
        if s.protect_todos && any(&TODO) {
            return Decision::Protected("TODO-style note");
        }
        if custom(&self.protect) {
            return Decision::Protected("your protect pattern");
        }

        if s.delete_empty && lines.is_empty() {
            return Decision::Delete("empty comment");
        }
        if s.delete_dividers && all(&DIVIDER) {
            return Decision::Delete("divider");
        }
        if s.delete_commented_code && all(&CODE_LINE) {
            return Decision::Delete("commented-out code");
        }
        if custom(&self.delete) {
            return Decision::Delete("your delete pattern");
        }

        let line_count = hunk.last - hunk.first + 1;
        if s.squash_enabled
            && line_count <= s.squash_max_lines as usize
            && (s.squash_doc_comments || !parsed.doc)
            && (s.squash_lists || !any(&LIST_ITEM))
            && let Some(text) = squash(hunk, syntax)
            && hunk.start + text.chars().count() <= s.squash_max_width as usize
        {
            return Decision::Squash(text);
        }
        Decision::Keep
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::highlight::tests::highlighter;
    use crate::highlight::{Document, Highlighter};
    use std::path::Path;

    fn decide_with(settings: &Settings, file: &str, source: &str) -> Decision {
        let h: &Highlighter = highlighter();
        let language = h.language_for(Path::new(file));
        let doc: Document = h.highlight(source, &language, "Dark+").unwrap();
        Rules::new(settings).decide(&doc.hunks[0], h.syntax_for(&language))
    }

    fn decide(file: &str, source: &str) -> Decision {
        decide_with(&Settings::default(), file, source)
    }

    #[test]
    fn a_typed_comment_gets_the_verdict_the_same_comment_would_in_a_file() {
        let h = highlighter();
        let syntax = h.syntax_for("javascript");
        let mut settings = Settings::default();
        settings.delete_patterns = vec!["^// wip".to_owned()];
        let rules = Rules::new(&settings);
        assert_eq!(try_comment(&rules, "// -----", syntax), Some(Decision::Delete("divider")));
        assert_eq!(try_comment(&rules, "// wip again", syntax), Some(Decision::Delete("your delete pattern")));
        assert_eq!(try_comment(&rules, "// a\n// b", syntax), Some(Decision::Squash("// a b".to_owned())));
        assert_eq!(try_comment(&rules, "/* TODO later */", syntax), Some(Decision::Protected("TODO-style note")));
        assert_eq!(try_comment(&rules, "not a comment", syntax), None);
    }

    #[test]
    fn directives_are_protected() {
        for (file, source) in [
            ("x.ts", "// eslint-disable-next-line no-console\n"),
            ("x.ts", "// @ts-expect-error nope\n"),
            ("x.ts", "/* istanbul ignore next */\n"),
            ("x.py", "# type: ignore\n"),
            ("x.py", "# noqa: E501\n"),
            ("x.go", "//go:build linux\n"),
            ("x.sh", "#!/bin/bash\n"),
            ("x.ts", "// #region helpers\n"),
        ] {
            assert_eq!(decide(file, source), Decision::Protected("tool directive"), "{source}");
        }
    }

    #[test]
    fn directives_can_be_unprotected() {
        let settings = Settings { protect_directives: false, ..Default::default() };
        assert_eq!(decide_with(&settings, "x.ts", "// eslint-disable\n"), Decision::Keep);
    }

    #[test]
    fn licences_and_todos_are_protected() {
        assert!(decide("x.rs", "// Copyright 2026 Someone\n").is_protected());
        assert!(decide("x.rs", "// SPDX-License-Identifier: MIT\n").is_protected());
        assert!(decide("x.rs", "// TODO: later\n// more\n").is_protected());
    }

    #[test]
    fn ordinary_words_are_not_mistaken_for_directives() {
        assert_eq!(decide("x.rs", "// the todo list lives here\n"), Decision::Keep);
        assert_eq!(decide("x.rs", "// going to refactor this\n"), Decision::Keep);
    }

    #[test]
    fn empty_comments_and_dividers_are_deleted() {
        assert_eq!(decide("x.rs", "//\n"), Decision::Delete("empty comment"));
        assert_eq!(decide("x.ts", "/* */\n"), Decision::Delete("empty comment"));
        assert_eq!(decide("x.rs", "// ---------\n"), Decision::Delete("divider"));
        assert_eq!(decide("x.py", "# =====\n"), Decision::Delete("divider"));
    }

    #[test]
    fn a_titled_divider_is_kept() {
        assert_eq!(decide("x.rs", "// --- Parsing ---\n"), Decision::Keep);
    }

    #[test]
    fn short_runs_squash() {
        assert_eq!(decide("x.rs", "// one\n// two\n"), Decision::Squash("// one two".into()));
    }

    #[test]
    fn squash_respects_width_and_line_limits() {
        let long = format!("// {}\n// {}\n", "a".repeat(100), "b".repeat(100));
        assert_eq!(decide("x.rs", &long), Decision::Keep);
        let wide = Settings { squash_max_width: 250, ..Default::default() };
        assert!(matches!(decide_with(&wide, "x.rs", &long), Decision::Squash(_)));
        assert_eq!(decide("x.rs", "// a\n// b\n// c\n// d\n// e\n"), Decision::Keep);
    }

    #[test]
    fn doc_comments_and_lists_do_not_squash_by_default() {
        assert_eq!(decide("x.rs", "/// one\n/// two\nfn f() {}\n"), Decision::Keep);
        assert_eq!(decide("x.ts", "/**\n * one\n * two\n */\nf();\n"), Decision::Keep);
        assert_eq!(decide("x.rs", "// - one\n// - two\n"), Decision::Keep);
        let on = Settings { squash_doc_comments: true, squash_lists: true, ..Default::default() };
        assert!(decide_with(&on, "x.rs", "/// one\n/// two\nfn f() {}\n").is_fix());
        assert!(decide_with(&on, "x.rs", "// - one\n// - two\n").is_fix());
    }

    #[test]
    fn commented_out_code_is_opt_in() {
        assert_eq!(decide("x.ts", "// const a = 1;\n"), Decision::Keep);
        let on = Settings { delete_commented_code: true, ..Default::default() };
        assert_eq!(decide_with(&on, "x.ts", "// const a = 1;\n"), Decision::Delete("commented-out code"));
        assert_eq!(decide_with(&on, "x.py", "# foo(bar)\n"), Decision::Delete("commented-out code"));
    }

    #[test]
    fn prose_is_not_commented_out_code() {
        let on = Settings { delete_commented_code: true, ..Default::default() };
        for source in [
            "// import this file before the others\n",
            "// return the value to the caller\n",
            "// this sets x = 1 for later\n",
            "// see the docs (section two)\n",
            "// from here on it gets messy\n",
        ] {
            assert_eq!(decide_with(&on, "x.ts", source), Decision::Keep, "{source}");
        }
    }

    #[test]
    fn custom_patterns_work_and_protection_wins() {
        let settings = Settings {
            delete_patterns: vec!["^generated".into(), "^secret".into()],
            protect_patterns: vec!["secret".into()],
            ..Default::default()
        };
        assert_eq!(decide_with(&settings, "x.rs", "// generated by tool\n"), Decision::Delete("your delete pattern"));
        assert!(decide_with(&settings, "x.rs", "// secret sauce\n").is_protected());
    }

    #[test]
    fn bad_patterns_are_reported_not_fatal() {
        let settings = Settings { delete_patterns: vec!["(".into()], ..Default::default() };
        assert_eq!(Rules::new(&settings).errors, vec!["(".to_string()]);
    }

    #[test]
    fn settings_files_tolerate_missing_fields() {
        let settings: Settings = serde_json::from_str(r#"{"squash_max_width": 80}"#).unwrap();
        assert_eq!(settings.squash_max_width, 80);
        assert!(settings.protect_directives);
    }
}
