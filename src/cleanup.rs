//! Comment edits. Every edit replaces one hunk's region with new text, and is refused unless re-tokenizing shows the file's code is unchanged.

use crate::files::{self, Loaded};
use crate::highlight::{Document, Highlighter, Hunk, HunkKind, line_prefix};
use crate::vscode::CommentSyntax;
use std::path::Path;
use std::sync::Arc;

/// A comment split into its delimiters and its text.
pub struct Parsed<'a> {
    pub opener: String,
    pub closer: Option<&'a str>,
    pub lines: Vec<&'a str>,
    pub doc: bool,
}

pub fn parse<'a>(hunk: &'a Hunk, syntax: Option<&'a CommentSyntax>) -> Option<Parsed<'a>> {
    match hunk.kind {
        HunkKind::LineRun => {
            let delimiter = syntax?.line.as_deref()?;
            let prefix = line_prefix(&hunk.raw, delimiter);
            Some(Parsed {
                opener: prefix.to_owned(),
                closer: None,
                lines: hunk
                    .raw
                    .lines()
                    .map(|l| l.strip_prefix(prefix).unwrap_or(l).trim())
                    .collect(),
                doc: prefix != delimiter,
            })
        }
        HunkKind::Block => {
            let (open, close) = syntax?.block.as_ref()?;
            let inner = hunk.raw.strip_prefix(open.as_str())?.strip_suffix(close.as_str())?;
            let decor = open.chars().last().filter(|c| !c.is_alphanumeric())?;
            let extra: usize = inner.chars().take_while(|c| *c == decor).count() * decor.len_utf8();
            let opener = format!("{open}{}", &inner[..extra]);
            let inner = inner[extra..].trim_end_matches(decor);
            Some(Parsed {
                opener,
                closer: Some(close.as_str()),
                lines: inner
                    .lines()
                    .map(|l| {
                        let l = l.trim();
                        l.strip_prefix(decor).unwrap_or(l).trim()
                    })
                    .collect(),
                doc: extra > 0,
            })
        }
        HunkKind::Unknown => None,
    }
}

/// One comment line for a multi-line hunk, or `None` if it can't be squashed.
pub fn squash(hunk: &Hunk, syntax: Option<&CommentSyntax>) -> Option<String> {
    if hunk.first == hunk.last {
        return None;
    }
    let parsed = parse(hunk, syntax)?;
    let text = parsed
        .lines
        .iter()
        .copied()
        .filter(|l| !l.is_empty())
        .collect::<Vec<_>>()
        .join(" ");
    if text.is_empty() {
        return None;
    }
    Some(match parsed.closer {
        Some(close) => format!("{} {text} {close}", parsed.opener),
        None => format!("{} {text}", parsed.opener),
    })
}

#[derive(Clone, Debug, PartialEq)]
pub enum Change {
    Replace(String),
    Delete,
}

/// (start, end) byte offsets of each line's content, for any line-ending style.
fn line_spans(body: &str) -> Vec<(usize, usize)> {
    let bytes = body.as_bytes();
    let mut spans = Vec::new();
    let (mut start, mut i) = (0, 0);
    while i < bytes.len() {
        match bytes[i] {
            b'\n' => {
                spans.push((start, i));
                start = i + 1;
            }
            b'\r' => {
                spans.push((start, i));
                if bytes.get(i + 1) == Some(&b'\n') {
                    i += 1;
                }
                start = i + 1;
            }
            _ => {}
        }
        i += 1;
    }
    spans.push((start, bytes.len()));
    spans
}

fn newline_of(body: &str) -> &'static str {
    match body.find(['\n', '\r']) {
        Some(i) if body[i..].starts_with("\r\n") => "\r\n",
        Some(i) if body[i..].starts_with('\r') => "\r",
        _ => "\n",
    }
}

/// The hunk's byte range in `body`, after checking it still matches.
fn locate(body: &str, spans: &[(usize, usize)], hunk: &Hunk) -> Result<(usize, usize), String> {
    let stale = || "The file doesn't match what's on screen. Try again.".to_owned();
    let &(first_start, first_end) = spans.get(hunk.first).ok_or_else(stale)?;
    let &(last_start, last_end) = spans.get(hunk.last).ok_or_else(stale)?;
    let start = first_start + hunk.start;
    let end = last_start + hunk.end;
    let first_piece = hunk.raw.lines().next().unwrap_or("");
    if end > last_end || start > first_end || !body.is_char_boundary(start) || !body.is_char_boundary(end) {
        return Err(stale());
    }
    if !body[start..].starts_with(first_piece) {
        return Err(stale());
    }
    Ok((start, end))
}

/// Removes a comment. A comment alone on its lines takes those lines with it, and a blank line is dropped too if the removal would leave two in a row.
pub fn delete_hunk(body: &str, hunk: &Hunk) -> Result<(String, (usize, usize)), String> {
    let spans = line_spans(body);
    let (start, end) = locate(body, &spans, hunk)?;
    let (first_start, _) = spans[hunk.first];
    let (_, last_end) = spans[hunk.last];
    let before = &body[first_start..start];
    let after = &body[end..last_end];
    let blank = |line: usize| spans.get(line).is_some_and(|&(s, e)| body[s..e].trim().is_empty());

    let (cut_from, cut_to, column) = if before.trim().is_empty() && after.trim().is_empty() {
        match spans.get(hunk.last + 1) {
            Some(&(next_start, _)) => {
                let previous_blank = hunk.first == 0 || blank(hunk.first - 1);
                let to = match spans.get(hunk.last + 2) {
                    Some(&(after_next, _)) if previous_blank && blank(hunk.last + 1) => after_next,
                    _ => next_start,
                };
                (first_start, to, 0)
            }
            None => (hunk.first.checked_sub(1).map_or(0, |p| spans[p].1), body.len(), 0),
        }
    } else if after.trim().is_empty() {
        let kept = before.trim_end().len();
        (first_start + kept, last_end, kept)
    } else {
        (start, end, hunk.start)
    };
    let new_body = format!("{}{}", &body[..cut_from], &body[cut_to..]);
    Ok((new_body, (hunk.first, column)))
}

/// Later lines of a multi-line replacement get the hunk's indent.
fn indented(hunk: &Hunk, first_line: &str, replacement: &str) -> String {
    let indent = if hunk.whole_line { &first_line[..hunk.start] } else { "" };
    replacement
        .lines()
        .enumerate()
        .map(|(i, line)| match i {
            0 => line.to_owned(),
            _ if line.trim().is_empty() => String::new(),
            _ => format!("{indent}{line}"),
        })
        .collect::<Vec<_>>()
        .join("\n")
}

/// Returns the new file text and where the replacement ends, as (line, byte column).
pub fn replace_hunk(
    body: &str,
    hunk: &Hunk,
    replacement: &str,
) -> Result<(String, (usize, usize)), String> {
    let spans = line_spans(body);
    let (start, end) = locate(body, &spans, hunk)?;
    let (first_start, first_end) = spans[hunk.first];

    let text = indented(hunk, &body[first_start..first_end], replacement);
    let resume = match text.rsplit_once('\n') {
        Some((before, tail)) => (hunk.first + before.matches('\n').count() + 1, tail.len()),
        None => (hunk.first, hunk.start + text.len()),
    };
    let text = text.replace('\n', newline_of(body));
    Ok((format!("{}{text}{}", &body[..start], &body[end..]), resume))
}

/// The file's code with all whitespace removed. Equal fingerprints mean an edit changed nothing but comments.
pub fn code_fingerprint(doc: &Document) -> String {
    doc.lines
        .iter()
        .flat_map(|line| &line.segments)
        .filter(|segment| !segment.comment)
        .flat_map(|segment| segment.text.chars())
        .filter(|c| !c.is_whitespace())
        .collect()
}

/// Whether replacing a comment with this text would leave the code untouched.
pub fn check_replacement(
    highlighter: &Highlighter,
    theme: &str,
    current: &Document,
    hunk: &Hunk,
    replacement: &str,
) -> Result<(), String> {
    let snapshot = current
        .lines
        .iter()
        .map(|line| {
            let text: String = line.segments.iter().map(|segment| segment.text.as_str()).collect();
            text.trim_end_matches(['\r', '\n']).to_owned()
        })
        .collect::<Vec<_>>()
        .join("\n");
    let (body, _) = replace_hunk(&snapshot, hunk, replacement)?;
    let new_doc = highlighter.highlight(&body, &current.language, theme)?;
    if code_fingerprint(&new_doc) != code_fingerprint(current) {
        return Err("The suggestion wasn't a valid comment for this file. Try again.".into());
    }
    Ok(())
}

#[derive(Clone, Debug)]
pub struct Applied {
    pub loaded: Loaded,
    pub previous: Arc<Vec<u8>>,
    pub resume: (usize, usize),
    pub changed: usize,
}

pub fn apply_changes(
    highlighter: &Highlighter,
    path: &Path,
    theme: &str,
    expected_hash: u64,
    current: &Document,
    changes: &[(Hunk, Change)],
) -> Result<Applied, String> {
    let bytes = std::fs::read(path).map_err(|e| format!("Couldn't read the file: {e}"))?;
    if files::hash_of(&bytes) != expected_hash {
        return Err("The file changed on disk. Nothing was written; try again.".into());
    }
    let decoded = files::decode(&bytes);
    if let Some(reason) = decoded.read_only {
        return Err(reason);
    }

    let mut ordered: Vec<&(Hunk, Change)> = changes.iter().collect();
    ordered.sort_by_key(|(hunk, _)| std::cmp::Reverse((hunk.first, hunk.start)));
    let mut new_body = decoded.body;
    let mut resume = (0, 0);
    for (hunk, change) in ordered {
        (new_body, resume) = match change {
            Change::Replace(text) => replace_hunk(&new_body, hunk, text)?,
            Change::Delete => delete_hunk(&new_body, hunk)?,
        };
    }

    let new_doc = highlighter.highlight(&new_body, &current.language, theme)?;
    if code_fingerprint(&new_doc) != code_fingerprint(current) {
        return Err(
            "Refused: that would change code, not just the comment. Keep the comment \
             delimiters in place. Nothing was written."
                .into(),
        );
    }

    let new_bytes = files::encode(&new_body, decoded.bom);
    files::write_atomic(path, &new_bytes)?;
    Ok(Applied {
        loaded: Loaded {
            doc: Arc::new(new_doc),
            hash: files::hash_of(&new_bytes),
            read_only: None,
        },
        previous: Arc::new(bytes),
        resume,
        changed: changes.len(),
    })
}

pub fn undo(
    highlighter: &Highlighter,
    path: &Path,
    theme: &str,
    expected_hash: u64,
    previous: &[u8],
) -> Result<Loaded, String> {
    let bytes = std::fs::read(path).map_err(|e| format!("Couldn't read the file: {e}"))?;
    if files::hash_of(&bytes) != expected_hash {
        return Err("The file changed on disk since that edit, so it can't be undone.".into());
    }
    files::write_atomic(path, previous)?;
    let decoded = files::decode(previous);
    let doc = highlighter.highlight(&decoded.body, &highlighter.language_for(path), theme)?;
    Ok(Loaded {
        doc: Arc::new(doc),
        hash: files::hash_of(previous),
        read_only: decoded.read_only,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::highlight::tests::highlighter;
    use std::path::PathBuf;

    fn doc(file: &str, source: &str) -> Document {
        let h = highlighter();
        h.highlight(source, &h.language_for(Path::new(file)), "Dark+").unwrap()
    }

    fn syntax(language: &str) -> Option<&'static CommentSyntax> {
        highlighter().syntax_for(language)
    }

    #[test]
    fn squashes_a_line_run() {
        let d = doc("x.rs", "    // one\n    //\n    //   two\nlet x = 1;\n");
        assert_eq!(squash(&d.hunks[0], syntax("rust")).unwrap(), "// one two");
    }

    #[test]
    fn squash_keeps_doc_prefix() {
        let d = doc("x.rs", "/// one\n/// two\nfn f() {}\n");
        assert_eq!(squash(&d.hunks[0], syntax("rust")).unwrap(), "/// one two");
    }

    #[test]
    fn squashes_a_jsdoc_block() {
        let d = doc("x.ts", "/**\n * One.\n * @param a two\n */\nf();\n");
        assert_eq!(squash(&d.hunks[0], syntax("typescript")).unwrap(), "/** One. @param a two */");
    }

    #[test]
    fn squashes_an_html_block() {
        let d = doc("x.html", "<!-- one\n  two -->\n<p></p>\n");
        assert_eq!(squash(&d.hunks[0], syntax("html")).unwrap(), "<!-- one two -->");
    }

    #[test]
    fn single_lines_cannot_be_squashed() {
        let d = doc("x.rs", "// one\n");
        assert_eq!(squash(&d.hunks[0], syntax("rust")), None);
    }

    fn deleted(file: &str, source: &str, index: usize) -> String {
        delete_hunk(source, &doc(file, source).hunks[index]).unwrap().0
    }

    #[test]
    fn delete_removes_whole_comment_lines() {
        assert_eq!(deleted("x.rs", "a();\n  // one\n  // two\nb();\n", 0), "a();\nb();\n");
    }

    #[test]
    fn delete_keeps_crlf() {
        assert_eq!(deleted("x.rs", "a();\r\n// one\r\nb();\r\n", 0), "a();\r\nb();\r\n");
    }

    #[test]
    fn delete_trailing_comment_keeps_the_code() {
        assert_eq!(deleted("x.rs", "let x = 1;   // old\ny();\n", 0), "let x = 1;\ny();\n");
    }

    #[test]
    fn delete_inline_comment_keeps_code_on_both_sides() {
        assert_eq!(deleted("x.c", "int a /* b */ = 1;\n", 0), "int a  = 1;\n");
    }

    #[test]
    fn delete_avoids_doubling_blank_lines() {
        assert_eq!(deleted("x.rs", "a();\n\n// c\n\nb();\n", 0), "a();\n\nb();\n");
    }

    #[test]
    fn delete_at_start_of_file_leaves_no_blank_top_line() {
        assert_eq!(deleted("x.rs", "// c\n\nb();\n", 0), "b();\n");
    }

    #[test]
    fn delete_last_line_without_newline() {
        assert_eq!(deleted("x.rs", "a();\n// c", 0), "a();");
    }

    #[test]
    fn delete_one_of_two_comments_on_a_line() {
        assert_eq!(deleted("x.c", "/* a */ x /* b */\n", 0), " x /* b */\n");
    }

    #[test]
    fn apply_many_changes_in_one_write() {
        let source = "// a\n// b\nx();\n// c\ny();\nz(); // d\n";
        let file = Scratch::new("many.rs", source.as_bytes());
        let d = doc("x.rs", source);
        let changes = vec![
            (d.hunks[2].clone(), Change::Delete),
            (d.hunks[0].clone(), Change::Replace("// a b".into())),
            (d.hunks[1].clone(), Change::Delete),
        ];
        let hash = files::hash_of(source.as_bytes());
        let applied = apply_changes(highlighter(), &file.0, "Dark+", hash, &d, &changes).unwrap();
        assert_eq!(file.read(), "// a b\nx();\ny();\nz();\n");
        assert_eq!(applied.changed, 3);
        undo(highlighter(), &file.0, "Dark+", applied.loaded.hash, &applied.previous).unwrap();
        assert_eq!(file.read(), source);
    }

    fn replaced(file: &str, source: &str, index: usize, text: &str) -> String {
        let d = doc(file, source);
        replace_hunk(source, &d.hunks[index], text).unwrap().0
    }

    #[test]
    fn replace_collapses_lines_and_keeps_crlf() {
        assert_eq!(
            replaced("x.rs", "fn a() {\r\n    // one\r\n    // two\r\n    b();\r\n}\r\n", 0, "// both"),
            "fn a() {\r\n    // both\r\n    b();\r\n}\r\n"
        );
    }

    #[test]
    fn replace_trailing_comment_leaves_code_alone() {
        assert_eq!(replaced("x.rs", "let x = 1; // old\n", 0, "// new"), "let x = 1; // new\n");
    }

    #[test]
    fn multi_line_reword_is_indented_with_file_newlines() {
        assert_eq!(
            replaced("x.rs", "    // old\r\nx();\r\n", 0, "// a\n// b"),
            "    // a\r\n    // b\r\nx();\r\n"
        );
    }

    #[test]
    fn replace_handles_non_ascii_before_the_comment() {
        assert_eq!(replaced("x.rs", "let s = \"héllo\"; // old\n", 0, "// new"), "let s = \"héllo\"; // new\n");
    }

    #[test]
    fn resume_points_at_end_of_replacement() {
        let source = "  // a\n  // b\nx();\n";
        let d = doc("x.rs", source);
        assert_eq!(replace_hunk(source, &d.hunks[0], "// c").unwrap().1, (0, 6));
        assert_eq!(replace_hunk(source, &d.hunks[0], "// c\n// de").unwrap().1, (1, 7));
    }

    #[test]
    fn stale_hunks_are_refused() {
        let d = doc("x.rs", "// old\n");
        assert!(replace_hunk("let different = 1;\n", &d.hunks[0], "// new").is_err());
        assert!(replace_hunk("", &doc("x.rs", "\n\n// old\n").hunks[0], "// new").is_err());
    }

    struct Scratch(PathBuf);
    impl Scratch {
        fn new(name: &str, bytes: &[u8]) -> Self {
            let dir = std::env::temp_dir().join(format!("commentcleanup-{}", std::process::id()));
            std::fs::create_dir_all(&dir).unwrap();
            let path = dir.join(name);
            std::fs::write(&path, bytes).unwrap();
            Self(path)
        }
        fn read(&self) -> String {
            String::from_utf8(std::fs::read(&self.0).unwrap()).unwrap()
        }
    }
    impl Drop for Scratch {
        fn drop(&mut self) {
            let _ = std::fs::remove_file(&self.0);
        }
    }

    fn edit(file: &Scratch, source: &str, text: &str) -> Result<Applied, String> {
        let h = highlighter();
        let d = doc(file.0.to_str().unwrap(), source);
        let changes = [(d.hunks[0].clone(), Change::Replace(text.into()))];
        apply_changes(h, &file.0, "Dark+", files::hash_of(source.as_bytes()), &d, &changes)
    }

    #[test]
    fn apply_writes_and_undo_restores() {
        let source = "// a\n// b\nlet x = 1;\n";
        let file = Scratch::new("apply.rs", source.as_bytes());
        let applied = edit(&file, source, "// a b").unwrap();
        assert_eq!(file.read(), "// a b\nlet x = 1;\n");
        undo(highlighter(), &file.0, "Dark+", applied.loaded.hash, &applied.previous).unwrap();
        assert_eq!(file.read(), source);
    }

    #[test]
    fn rewording_into_code_is_refused() {
        let source = "// a\nlet x = 1;\n";
        let file = Scratch::new("code.rs", source.as_bytes());
        assert!(edit(&file, source, "let y = 2;").unwrap_err().starts_with("Refused"));
        assert_eq!(file.read(), source);
    }

    #[test]
    fn opening_a_block_that_swallows_code_is_refused() {
        let source = "// a\nlet x = 1;\n";
        let file = Scratch::new("swallow.rs", source.as_bytes());
        assert!(edit(&file, source, "/* a").unwrap_err().starts_with("Refused"));
        assert_eq!(file.read(), source);
    }

    #[test]
    fn external_changes_block_the_write() {
        let source = "// a\n";
        let file = Scratch::new("raced.rs", b"// changed by someone else\n");
        assert!(edit(&file, source, "// b").unwrap_err().contains("changed on disk"));
        assert_eq!(file.read(), "// changed by someone else\n");
    }

    #[test]
    fn bom_survives_an_edit() {
        let bytes = "\u{feff}// a\n".as_bytes();
        let file = Scratch::new("bom.rs", bytes);
        let h = highlighter();
        let d = doc("x.rs", "// a\n");
        let changes = [(d.hunks[0].clone(), Change::Replace("// b".into()))];
        apply_changes(h, &file.0, "Dark+", files::hash_of(bytes), &d, &changes).unwrap();
        assert_eq!(file.read(), "\u{feff}// b\n");
    }
}
