//! Turns a file into coloured segments and comment hunks in a single pass

use crate::bundled;
use crate::vscode::{self, Catalog, CommentSyntax, IconThemeEntry, ThemeColors};
use giallo::{FontStyle, HighlightOptions, Registry, ThemeVariant};
use std::collections::{HashMap, HashSet};
use std::ops::Range;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::path::Path;
use std::time::{Duration, Instant};

const PROBE_THEME: &str = "__comment_probe__";
const PROBE_COMMENT: &str = "#c0ffee";
const PROBE_JSON: &str = r##"{
  "name": "__comment_probe__",
  "type": "dark",
  "colors": { "editor.foreground": "#000001", "editor.background": "#000000" },
  "tokenColors": [
    { "scope": ["comment", "punctuation.definition.comment"],
      "settings": { "foreground": "#c0ffee" } }
  ]
}"##;

const BLOCKED_GRAMMARS: &[&str] = &["snippets"];

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Rgba(pub f32, pub f32, pub f32, pub f32);

pub fn parse_hex(hex: &str) -> Rgba {
    let h = hex.trim_start_matches('#');
    let channel = |i: usize| u8::from_str_radix(h.get(i..i + 2).unwrap_or("ff"), 16).unwrap_or(255);
    let short = |i: usize| u8::from_str_radix(h.get(i..i + 1).unwrap_or("f"), 16).unwrap_or(15) * 17;
    let (r, g, b, a) = match h.len() {
        3 => (short(0), short(1), short(2), 255),
        4 => (short(0), short(1), short(2), short(3)),
        6 => (channel(0), channel(2), channel(4), 255),
        _ => (channel(0), channel(2), channel(4), channel(6)),
    };
    Rgba(
        r as f32 / 255.0,
        g as f32 / 255.0,
        b as f32 / 255.0,
        a as f32 / 255.0,
    )
}

#[derive(Clone, Debug)]
pub struct Segment {
    pub text: String,
    pub color: Rgba,
    pub bold: bool,
    pub italic: bool,
    pub comment: bool,
}

#[derive(Clone, Debug, Default)]
pub struct Line {
    pub segments: Vec<Segment>,
    pub chars: usize,
}

impl Line {
    pub fn text(&self) -> String {
        self.segments.iter().map(|s| s.text.as_str()).collect()
    }

    pub fn has_code(&self) -> bool {
        self.segments
            .iter()
            .any(|s| !s.comment && !s.text.trim().is_empty())
    }

    fn has_comment(&self) -> bool {
        self.segments
            .iter()
            .any(|s| s.comment && !s.text.trim().is_empty())
    }

    /// Byte ranges of each comment on this line, without surrounding whitespace.
    fn comment_pieces(&self, text: &str) -> Vec<Range<usize>> {
        let mut pieces: Vec<Range<usize>> = Vec::new();
        let mut offset = 0;
        for segment in &self.segments {
            let end = offset + segment.text.len();
            if segment.comment {
                match pieces.last_mut() {
                    Some(last) if last.end == offset => last.end = end,
                    _ => pieces.push(offset..end),
                }
            }
            offset = end;
        }
        pieces
            .into_iter()
            .filter_map(|range| {
                let slice = &text[range.clone()];
                let trimmed = slice.trim();
                if trimmed.is_empty() {
                    return None;
                }
                let start = range.start + (slice.len() - slice.trim_start().len());
                Some(start..start + trimmed.len())
            })
            .collect()
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum HunkKind {
    /// Adjacent `//`-style lines with the same indent and prefix.
    LineRun,
    /// One `/* */`-style comment, possibly over several lines.
    Block,
    /// A comment whose delimiters the language configuration doesn't describe, such as one in an embedded language.
    Unknown,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Hunk {
    pub first: usize,
    pub last: usize,
    /// Byte column in `first` where the comment starts.
    pub start: usize,
    /// Byte column in `last` where the comment ends.
    pub end: usize,
    pub kind: HunkKind,
    /// No code shares the first line.
    pub whole_line: bool,
    /// The comment text, one line per source line, trimmed.
    pub raw: String,
    /// Identifies this hunk across reloads: its text plus which occurrence it is.
    pub key: u64,
}

pub struct Document {
    pub lines: Vec<Line>,
    pub hunks: Vec<Hunk>,
    /// Hunk indexes on each line.
    pub line_hunks: Vec<Vec<u32>>,
    pub max_chars: usize,
    pub language: String,
    pub tokenize_time: Duration,
}

impl std::fmt::Debug for Document {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "Document({} lines)", self.lines.len())
    }
}

impl Document {
    pub fn empty() -> Self {
        Self {
            lines: Vec::new(),
            hunks: Vec::new(),
            line_hunks: Vec::new(),
            max_chars: 0,
            language: "plain".into(),
            tokenize_time: Duration::ZERO,
        }
    }
}

pub struct LoadReport {
    pub grammar_errors: Vec<String>,
    pub theme_errors: Vec<String>,
    pub elapsed: Duration,
}

pub struct Highlighter {
    registry: Registry,
    extensions: HashMap<String, String>,
    filenames: HashMap<String, String>,
    comments: HashMap<String, CommentSyntax>,
    pub palettes: HashMap<String, ThemeColors>,
    pub theme_names: Vec<String>,
    pub icon_themes: Vec<IconThemeEntry>,
}

impl std::fmt::Debug for Highlighter {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "Highlighter({} themes)", self.theme_names.len())
    }
}

impl Highlighter {
    pub fn load(catalog: Catalog) -> Result<(Self, LoadReport), String> {
        let start = Instant::now();
        let mut registry = Registry::builtin().map_err(|e| e.to_string())?;
        for (grammar, vs_code_id) in [
            ("jsx", "javascriptreact"),
            ("tsx", "typescriptreact"),
            ("razor", "cshtml"),
        ] {
            registry.add_alias(grammar, vs_code_id);
        }

        let mut grammar_errors = Vec::new();
        for entry in &catalog.grammars {
            if BLOCKED_GRAMMARS.contains(&entry.name.as_str())
                || registry.contains_grammar(&entry.name)
            {
                continue;
            }
            let result = catch_unwind(AssertUnwindSafe(|| {
                vscode::grammar_bytes(entry)
                    .and_then(|b| registry.add_grammar_from_bytes(&b).map_err(|e| e.to_string()))
            }))
            .unwrap_or_else(|_| Err("giallo could not load this grammar".into()));
            if let Err(e) = result {
                grammar_errors.push(format!("{}: {e}", entry.name));
            }
        }
        registry.link_grammars();

        let mut palettes = HashMap::new();
        let mut theme_names = Vec::new();
        let mut theme_errors = Vec::new();
        for entry in &catalog.themes {
            let result = vscode::flatten_theme(entry).and_then(|(bytes, palette)| {
                registry
                    .add_theme_from_bytes(&bytes)
                    .map_err(|e| e.to_string())?;
                Ok(palette)
            });
            match result {
                Ok(palette) => {
                    if palettes.insert(entry.label.to_lowercase(), palette).is_none() {
                        theme_names.push(entry.label.clone());
                    }
                }
                Err(e) => theme_errors.push(format!("{}: {e}", entry.label)),
            }
        }

        let installed: HashSet<String> =
            theme_names.iter().map(|name| bundled::theme_key(name)).collect();
        for id in bundled::THEMES {
            let label = bundled::theme_label(id);
            if installed.contains(&bundled::theme_key(&label)) {
                continue;
            }
            if let Some(palette) = bundled_palette(&registry, id) {
                palettes.insert(label.to_lowercase(), palette);
                theme_names.push(label);
            }
        }
        theme_names.sort_by_key(|name| name.to_lowercase());
        registry
            .add_theme_from_bytes(PROBE_JSON.as_bytes())
            .map_err(|e| e.to_string())?;

        let report = LoadReport {
            grammar_errors,
            theme_errors,
            elapsed: start.elapsed(),
        };
        Ok((
            Self {
                registry,
                extensions: catalog.extensions,
                filenames: catalog.filenames,
                comments: catalog.comments,
                palettes,
                theme_names,
                icon_themes: catalog.icon_themes,
            },
            report,
        ))
    }

    /// The VS Code language id for a path, or "plain" if it isn't recognised.
    pub fn language_for(&self, path: &Path) -> String {
        let by_name = path
            .file_name()
            .and_then(|n| n.to_str())
            .and_then(|n| self.filenames.get(&n.to_lowercase()));
        let by_extension = || {
            let extension = path.extension()?.to_str()?;
            self.extensions.get(&format!(".{}", extension.to_lowercase()))
        };
        by_name
            .or_else(by_extension)
            .cloned()
            .unwrap_or_else(|| "plain".to_owned())
    }

    pub fn syntax_for(&self, language: &str) -> Option<&CommentSyntax> {
        self.comments.get(language)
    }

    pub fn highlight(&self, source: &str, language: &str, theme: &str) -> Result<Document, String> {
        let start = Instant::now();
        let theme = self
            .palettes
            .get(&theme.to_lowercase())
            .map_or(theme, |palette| palette.id.as_str());
        let options = HighlightOptions::new(
            language,
            ThemeVariant::Dual {
                light: theme,
                dark: PROBE_THEME,
            },
        )
        .merge_whitespace(false)
        .fallback_to_plain(true);

        let code = self
            .registry
            .highlight(source, &options)
            .map_err(|e| e.to_string())?;

        let probe = giallo::Color::from_hex(PROBE_COMMENT).map_err(|e| e.to_string())?;
        let mut colors: HashMap<giallo::Color, Rgba> = HashMap::new();
        let mut lines = Vec::with_capacity(code.tokens.len());
        let mut max_chars = 0;

        for tokens in code.tokens {
            let mut line = Line::default();
            for token in tokens {
                let ThemeVariant::Dual { light, dark } = token.style else {
                    continue;
                };
                let comment = dark.foreground == probe;
                let color = *colors
                    .entry(light.foreground)
                    .or_insert_with(|| parse_hex(&light.foreground.as_hex()));
                let bold = light.font_style.contains(FontStyle::BOLD);
                let italic = light.font_style.contains(FontStyle::ITALIC);
                line.chars += token.text.chars().count();

                match line.segments.last_mut() {
                    Some(prev)
                        if prev.comment == comment
                            && prev.color == color
                            && prev.bold == bold
                            && prev.italic == italic =>
                    {
                        prev.text.push_str(&token.text);
                    }
                    _ => line.segments.push(Segment {
                        text: token.text,
                        color,
                        bold,
                        italic,
                        comment,
                    }),
                }
            }
            max_chars = max_chars.max(line.chars);
            lines.push(line);
        }

        let hunks = find_hunks(&lines, self.comments.get(language));
        let mut line_hunks = vec![Vec::new(); lines.len()];
        for (index, hunk) in hunks.iter().enumerate() {
            for slot in &mut line_hunks[hunk.first..=hunk.last] {
                slot.push(index as u32);
            }
        }

        Ok(Document {
            lines,
            hunks,
            line_hunks,
            max_chars,
            language: language.to_owned(),
            tokenize_time: start.elapsed(),
        })
    }
}

/// A bundled theme's colours, read from the compiled theme.
fn bundled_palette(registry: &Registry, id: &str) -> Option<ThemeColors> {
    let options = HighlightOptions::new("plain", ThemeVariant::Single(id));
    let code = registry.highlight("", &options).ok()?;
    let ThemeVariant::Single(theme) = code.theme else {
        return None;
    };
    Some(ThemeColors {
        id: id.to_owned(),
        foreground: theme.default_style.foreground.as_hex(),
        background: theme.default_style.background.as_hex(),
        line_number: theme
            .line_number_foreground
            .map_or_else(|| "#858585".to_owned(), |color| color.as_hex()),
    })
}

/// The delimiter plus any doc markers, so `///` and `//!` never merge with `//`.
pub fn line_prefix<'a>(body: &'a str, delimiter: &str) -> &'a str {
    let last = delimiter.chars().last().unwrap_or(' ');
    let rest = &body[delimiter.len()..];
    let extra: usize = rest
        .chars()
        .take_while(|c| *c == last || *c == '!')
        .map(char::len_utf8)
        .sum();
    &body[..delimiter.len() + extra]
}

/// FNV-1a. Hunk keys get saved to disk, so unlike the standard library's hasher this has to give the same answer in every future build.
fn stable_hash(text: &str) -> u64 {
    text.bytes().fold(0xcbf2_9ce4_8422_2325, |hash, byte| {
        (hash ^ u64::from(byte)).wrapping_mul(0x0000_0100_0000_01b3)
    })
}

fn find_hunks(lines: &[Line], syntax: Option<&CommentSyntax>) -> Vec<Hunk> {
    let mut hunks: Vec<Hunk> = Vec::new();
    let mut block_open = false;

    for (index, line) in lines.iter().enumerate() {
        if !line.has_comment() {
            continue;
        }
        let text = line.text();
        let whole_line = !line.has_code();

        for (position, piece) in line.comment_pieces(&text).into_iter().enumerate() {
            let body = &text[piece.clone()];

            if block_open {
                let hunk = hunks.last_mut().expect("an open block has a hunk");
                let at_line_start = position == 0 && text[..piece.start].trim().is_empty();
                let only_comment_between =
                    lines[hunk.last + 1..index].iter().all(|l| !l.has_code());
                if at_line_start && only_comment_between {
                    hunk.last = index;
                    hunk.end = piece.end;
                    hunk.raw.push('\n');
                    hunk.raw.push_str(body);
                    let close = syntax.and_then(|s| s.block.as_ref()).map(|b| b.1.as_str());
                    if close.is_none_or(|c| body.ends_with(c)) {
                        block_open = false;
                    }
                    continue;
                }
                block_open = false;
            }

            let block = syntax
                .and_then(|s| s.block.as_ref())
                .filter(|(open, _)| body.starts_with(open.as_str()));
            let line_delimiter = syntax
                .and_then(|s| s.line.as_deref())
                .filter(|d| body.starts_with(d));

            let kind = if let Some((open, close)) = block {
                block_open = !body[open.len()..].trim_end().ends_with(close.as_str());
                HunkKind::Block
            } else if line_delimiter.is_some() {
                HunkKind::LineRun
            } else {
                HunkKind::Unknown
            };

            if let (HunkKind::LineRun, Some(delimiter), Some(prev)) =
                (kind, line_delimiter, hunks.last_mut())
            {
                let same_run = whole_line
                    && prev.whole_line
                    && prev.kind == HunkKind::LineRun
                    && prev.last + 1 == index
                    && prev.start == piece.start
                    && line_prefix(&prev.raw, delimiter) == line_prefix(body, delimiter);
                if same_run {
                    prev.last = index;
                    prev.end = piece.end;
                    prev.raw.push('\n');
                    prev.raw.push_str(body);
                    continue;
                }
            }

            hunks.push(Hunk {
                first: index,
                last: index,
                start: piece.start,
                end: piece.end,
                kind,
                whole_line,
                raw: body.to_owned(),
                key: 0,
            });
        }
    }

    let mut seen: HashMap<u64, u64> = HashMap::new();
    for hunk in &mut hunks {
        let text_hash = stable_hash(&hunk.raw);
        let occurrence = seen.entry(text_hash).or_default();
        hunk.key = text_hash ^ occurrence.wrapping_mul(0x9E37_79B9_7F4A_7C15);
        *occurrence += 1;
    }
    hunks
}

#[cfg(test)]
pub mod tests {
    use super::*;
    use std::sync::OnceLock;

    pub fn highlighter() -> &'static Highlighter {
        static H: OnceLock<Highlighter> = OnceLock::new();
        H.get_or_init(|| Highlighter::load(vscode::scan(true)).unwrap().0)
    }

    #[test]
    fn hunk_keys_are_stable_between_builds() {
        assert_eq!(stable_hash(""), 0xcbf2_9ce4_8422_2325);
        assert_eq!(stable_hash("a"), 0xaf63_dc4c_8601_ec8c);
    }

    #[test]
    fn every_bundled_language_has_a_grammar() {
        let h = highlighter();
        for (id, ..) in bundled::LANGUAGES {
            assert!(h.registry.contains_grammar(id), "no grammar for {id}");
        }
    }

    #[test]
    fn every_bundled_theme_is_offered_or_replaced_by_an_installed_copy() {
        let h = highlighter();
        let offered: HashSet<String> =
            h.theme_names.iter().map(|name| bundled::theme_key(name)).collect();
        for id in bundled::THEMES {
            assert!(h.registry.contains_theme(id), "no theme {id}");
            assert!(offered.contains(&bundled::theme_key(&bundled::theme_label(id))), "{id} not offered");
        }
    }

    #[test]
    fn bundled_theme_highlights_comments() {
        let h = highlighter();
        let label = bundled::theme_label("tokyo-night");
        let doc = h.highlight("// hello\nlet a = 1;\n", "typescript", &label).unwrap();
        assert_eq!(doc.hunks.len(), 1);
        assert_eq!(doc.hunks[0].first, 0);
    }

    fn doc(file: &str, source: &str) -> Document {
        let h = highlighter();
        h.highlight(source, &h.language_for(Path::new(file)), "Dark+")
            .unwrap()
    }

    fn comments(ext: &str, source: &str) -> Vec<String> {
        doc(&format!("x.{ext}"), source)
            .hunks
            .iter()
            .flat_map(|h| h.raw.lines().map(str::to_owned).collect::<Vec<_>>())
            .collect()
    }

    macro_rules! case {
        ($name:ident, $ext:literal, $src:expr, [$($want:literal),* $(,)?]) => {
            #[test]
            fn $name() {
                let got = comments($ext, $src);
                let want: Vec<String> = vec![$($want.to_string()),*];
                assert_eq!(got, want, "\nsource:\n{}", $src);
            }
        };
    }

    case!(rust, "rs",
        "let u = \"http://x.y\"; // real\n/* block\n   two */\nlet r = r#\"// no\"#;\nlet c = '/';\n/// doc\nfn f() {}\n",
        ["// real", "/* block", "two */", "/// doc"]);
    case!(typescript, "ts",
        "const s = \"// no\"; // yes\nconst re = /\\/\\/ no/g;\nconst t = `// no ${a /* in */} // no`;\n",
        ["// yes", "/* in */"]);
    case!(python, "py",
        "s = \"# no\"  # yes\n\"\"\"\n# docstring, not a comment\n\"\"\"\n",
        ["# yes"]);
    case!(c, "c", "char *s = \"// no\"; /* yes */\n// two\n", ["/* yes */", "// two"]);
    case!(go, "go", "s := `// no` // yes\n", ["// yes"]);
    case!(java, "java", "String s = \"/* no */\"; // yes\n", ["// yes"]);
    case!(csharp, "cs", "var s = @\"// no\"; // yes\n", ["// yes"]);
    case!(shell, "sh", "echo \"# no\" # yes\necho ${#x}\n", ["# yes"]);
    case!(powershell, "ps1", "$s = \"# no\" # yes\n<# block #>\n", ["# yes", "<# block #>"]);
    case!(html, "html", "<a href=\"//no\"><!-- yes --></a>\n", ["<!-- yes -->"]);
    case!(css, "css", "a { b: url(//no); } /* yes */\n", ["/* yes */"]);
    case!(sql, "sql", "select '-- no'; -- yes\n", ["-- yes"]);
    case!(yaml, "yaml", "a: \"# no\" # yes\n", ["# yes"]);
    case!(lua, "lua", "s = \"-- no\" -- yes\n--[[ block ]]\n", ["-- yes", "--[[ block ]]"]);
    case!(ruby, "rb", "s = \"# no\" # yes\n", ["# yes"]);
    case!(php, "php", "<?php $s = \"// no\"; // yes\n", ["// yes"]);
    case!(jsx, "tsx", "const a = <b>// no</b>; // yes\n", ["// yes"]);

    #[test]
    fn dracula_colours_come_through() {
        // Dracula comes from an installed extension, which a build machine won't have.
        if !highlighter().palettes.contains_key("dracula theme") {
            return;
        }
        let h = highlighter();
        let doc = h.highlight("// c\nlet x = 1;\n", "rust", "Dracula Theme").unwrap();
        let segment = &doc.lines[0].segments[0];
        assert!(segment.comment);
        assert_eq!(segment.color, parse_hex("#6272A4"));
    }

    #[test]
    fn segments_reproduce_each_line_exactly() {
        let source = "\tlet  x = 1;  // a  b  \n\n  /* c */ y();\n";
        let d = doc("x.rs", source);
        for (line, expected) in d.lines.iter().zip(source.split('\n')) {
            assert_eq!(line.text(), expected);
        }
    }

    fn shape(d: &Document) -> Vec<(usize, usize, HunkKind, bool)> {
        d.hunks.iter().map(|h| (h.first, h.last, h.kind, h.whole_line)).collect()
    }

    #[test]
    fn hunks_with_crlf() {
        let d = doc("x.rs", "// a\r\n// b\r\n\r\nlet x = 1; // t\r\n// c\r\n");
        assert_eq!(shape(&d), vec![
            (0, 1, HunkKind::LineRun, true),
            (3, 3, HunkKind::LineRun, false),
            (4, 4, HunkKind::LineRun, true),
        ]);
    }

    #[test]
    fn doc_comments_and_indents_stay_separate() {
        let d = doc("x.rs", "// a\n/// b\n/// c\n    // d\n//e\n");
        assert_eq!(
            d.hunks.iter().map(|h| h.raw.as_str()).collect::<Vec<_>>(),
            vec!["// a", "/// b\n/// c", "// d", "//e"]
        );
    }

    #[test]
    fn block_starting_after_code_is_one_hunk() {
        let d = doc("x.c", "int a; /* one\n  two\n\n  three */ int b;\n// c\n");
        assert_eq!(shape(&d), vec![
            (0, 3, HunkKind::Block, false),
            (4, 4, HunkKind::LineRun, true),
        ]);
        assert_eq!(d.hunks[0].raw, "/* one\ntwo\nthree */");
    }

    #[test]
    fn two_comments_on_one_line_are_two_hunks() {
        let d = doc("x.c", "/* a */ int x; /* b */\n");
        assert_eq!(d.hunks.len(), 2);
        assert_eq!(d.line_hunks[0], vec![0, 1]);
        assert_ne!(d.hunks[0].key, d.hunks[1].key);
    }

    #[test]
    fn identical_comments_get_distinct_keys() {
        let d = doc("x.rs", "// same\nlet a = 1;\n// same\n");
        assert_ne!(d.hunks[0].key, d.hunks[1].key);
    }

    #[test]
    fn unrecognised_files_are_plain() {
        let h = highlighter();
        assert_eq!(h.language_for(Path::new("a.nothing")), "plain");
        assert!(doc("a.nothing", "// not known\n").hunks.is_empty());
    }
}
