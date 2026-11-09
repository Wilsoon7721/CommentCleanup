//! Find in the open file, and find across every file in the folder.

use crate::files;
use crate::highlight::Document;
use crate::scan;
use fancy_regex::Regex;
use iced::futures::Stream;
use iced::futures::channel::mpsc;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Mutex;
use std::time::{Duration, Instant};

pub const MAX_FILE_MATCHES: usize = 20_000;
pub const MAX_FOLDER_MATCHES: usize = 20_000;
const BATCH_INTERVAL: Duration = Duration::from_millis(100);
const SEARCH_DELAY: Duration = Duration::from_millis(150);
const PREVIEW_BEFORE: usize = 24;
const PREVIEW_AFTER: usize = 140;

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Query {
    pub text: String,
    pub match_case: bool,
    pub whole_word: bool,
    pub regex: bool,
}

impl Query {
    pub fn compile(&self) -> Result<Option<Arc<Regex>>, String> {
        if self.text.is_empty() {
            return Ok(None);
        }
        let body = if self.regex { self.text.clone() } else { fancy_regex::escape(&self.text).into_owned() };
        let body = if self.whole_word { format!(r"\b(?:{body})\b") } else { body };
        let source = if self.match_case { body } else { format!("(?i){body}") };
        match Regex::new(&source) {
            Ok(regex) => Ok(Some(Arc::new(regex))),
            Err(_) => Err("That isn't a valid regular expression.".to_owned()),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct FindMatch {
    pub line: u32,
    pub start: u32,
    pub end: u32,
}

fn each_match(regex: &Regex, text: &str, mut found: impl FnMut(usize, usize) -> bool) {
    for item in regex.find_iter(text) {
        let Ok(item) = item else {
            return;
        };
        if item.start() != item.end() && !found(item.start(), item.end()) {
            return;
        }
    }
}

pub fn in_document(doc: &Document, regex: &Regex) -> Vec<FindMatch> {
    let mut matches = Vec::new();
    let mut text = String::new();
    for (index, line) in doc.lines.iter().enumerate() {
        text.clear();
        text.extend(line.segments.iter().map(|segment| segment.text.as_str()));
        each_match(regex, text.trim_end_matches(['\r', '\n']), |start, end| {
            matches.push(FindMatch { line: index as u32, start: start as u32, end: end as u32 });
            matches.len() < MAX_FILE_MATCHES
        });
        if matches.len() >= MAX_FILE_MATCHES {
            break;
        }
    }
    matches
}

#[derive(Clone, Debug)]
pub struct SearchHit {
    pub line: u32,
    pub start: u32,
    pub before: String,
    pub matched: String,
    pub after: String,
}

#[derive(Clone, Debug)]
pub struct SearchFile {
    pub path: PathBuf,
    pub label: String,
    pub hits: Vec<SearchHit>,
}

fn floor_char(text: &str, mut index: usize) -> usize {
    while !text.is_char_boundary(index) {
        index -= 1;
    }
    index
}

fn hit(line_number: usize, text: &str, start: usize, end: usize) -> SearchHit {
    let from = text[..start]
        .char_indices()
        .rev()
        .take(PREVIEW_BEFORE)
        .last()
        .map_or(start, |(index, _)| index);
    let leading = text[from..start].trim_start();
    let before = if from > 0 { format!("…{leading}") } else { leading.to_owned() };
    let matched_end = floor_char(text, end.min(start + 400));
    let until = text[matched_end..]
        .char_indices()
        .nth(PREVIEW_AFTER)
        .map_or(text.len(), |(index, _)| matched_end + index);
    SearchHit {
        line: line_number as u32,
        start: start as u32,
        before,
        matched: text[start..matched_end].to_owned(),
        after: text[matched_end..until].to_owned(),
    }
}

fn search_file(root: &Path, path: &Path, regex: &Regex, budget: &AtomicUsize) -> Option<SearchFile> {
    let bytes = std::fs::read(path).ok()?;
    let decoded = files::decode(&bytes);
    if decoded.read_only.is_some() {
        return None;
    }
    let mut hits = Vec::new();
    for (number, line) in decoded.body.split('\n').enumerate() {
        let line = line.strip_suffix('\r').unwrap_or(line);
        each_match(regex, line, |start, end| {
            hits.push(hit(number, line, start, end));
            budget.fetch_sub(1, Ordering::Relaxed) > 1
        });
        if budget.load(Ordering::Relaxed) == 0 {
            break;
        }
    }
    (!hits.is_empty()).then(|| SearchFile {
        label: path.strip_prefix(root).unwrap_or(path).to_string_lossy().into_owned(),
        path: path.to_path_buf(),
        hits,
    })
}

/// Searches a folder on worker threads, yielding results in timed batches. Dropping the stream stops the workers.
pub fn search_folder(root: PathBuf, regex: Arc<Regex>) -> impl Stream<Item = Vec<SearchFile>> {
    let (sender, receiver) = mpsc::unbounded();
    std::thread::spawn(move || {
        std::thread::sleep(SEARCH_DELAY);
        if sender.is_closed() {
            return;
        }
        let paths = scan::candidates(&root);
        let next = AtomicUsize::new(0);
        let budget = AtomicUsize::new(MAX_FOLDER_MATCHES);
        let batch = Mutex::new((Vec::new(), Instant::now()));
        let workers = std::thread::available_parallelism().map_or(2, |n| n.get() / 2).clamp(1, 8);
        std::thread::scope(|scope| {
            for _ in 0..workers {
                scope.spawn(|| {
                    while let Some(path) = paths.get(next.fetch_add(1, Ordering::Relaxed)) {
                        if sender.is_closed() || budget.load(Ordering::Relaxed) == 0 {
                            break;
                        }
                        let Some(file) = search_file(&root, path, &regex, &budget) else {
                            continue;
                        };
                        let mut guard = batch.lock().unwrap();
                        guard.0.push(file);
                        if guard.1.elapsed() >= BATCH_INTERVAL {
                            let _ = sender.unbounded_send(std::mem::take(&mut guard.0));
                            guard.1 = Instant::now();
                        }
                    }
                });
            }
        });
        let _ = sender.unbounded_send(std::mem::take(&mut batch.lock().unwrap().0));
    });
    receiver
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::highlight::tests::highlighter;
    use iced::futures::StreamExt;
    use iced::futures::executor::block_on;
    use std::fs;

    fn query(text: &str) -> Query {
        Query { text: text.to_owned(), ..Query::default() }
    }

    fn regex_for(query: &Query) -> Arc<Regex> {
        query.compile().unwrap().unwrap()
    }

    fn doc(source: &str) -> Arc<Document> {
        Arc::new(highlighter().highlight(source, "javascript", "Dark+").unwrap())
    }

    #[test]
    fn plain_text_is_taken_literally_and_ignores_case_by_default() {
        let found = in_document(&doc("a.b(1);\nA.B(2);\naxb(3);\n"), &regex_for(&query("a.b(")));
        assert_eq!(found.iter().map(|m| m.line).collect::<Vec<_>>(), [0, 1]);
    }

    #[test]
    fn match_case_whole_word_and_regex_all_narrow_it_down() {
        let source = doc("cat Cat concat cat2\n");
        let case = Query { match_case: true, ..query("cat") };
        assert_eq!(in_document(&source, &regex_for(&case)).len(), 3);
        let word = Query { whole_word: true, ..query("cat") };
        assert_eq!(in_document(&source, &regex_for(&word)).len(), 2);
        let regex = Query { regex: true, ..query(r"cat\d") };
        let found = in_document(&source, &regex_for(&regex));
        assert_eq!((found[0].start, found[0].end), (15, 19));
    }

    #[test]
    fn matches_span_highlighted_segments_and_comments() {
        let found = in_document(&doc("const x = 1; // note\n"), &regex_for(&query("1; // no")));
        assert_eq!(found, [FindMatch { line: 0, start: 10, end: 18 }]);
    }

    #[test]
    fn an_empty_query_finds_nothing_and_a_bad_regex_says_so() {
        assert!(query("").compile().unwrap().is_none());
        assert!(Query { regex: true, ..query("(") }.compile().is_err());
    }

    #[test]
    fn a_pattern_that_can_match_nothing_is_only_reported_where_it_matches() {
        let regex = Query { regex: true, ..query("x*") };
        let found = in_document(&doc("axxb\n"), &regex_for(&regex));
        assert_eq!(found, [FindMatch { line: 0, start: 1, end: 3 }]);
    }

    #[test]
    fn previews_keep_multibyte_text_whole_and_trim_long_lines() {
        let line = format!("{}é target {}", "日本語".repeat(40), "é".repeat(400));
        let start = line.find("target").unwrap();
        let shown = hit(3, &line, start, start + 6);
        assert_eq!(shown.matched, "target");
        assert!(shown.before.starts_with('…'));
        assert!(shown.after.chars().count() <= PREVIEW_AFTER);
        assert_eq!(shown.line, 3);
    }

    #[test]
    fn a_folder_search_follows_gitignore_and_skips_binary_files() {
        let root = std::env::temp_dir().join(format!("commentcleanup-find-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(root.join("src")).unwrap();
        fs::create_dir_all(root.join("dist")).unwrap();
        fs::write(root.join(".gitignore"), "dist/\n").unwrap();
        fs::write(root.join("src/a.txt"), "first\nneedle here\r\nand needle again").unwrap();
        fs::write(root.join("dist/b.txt"), "needle").unwrap();
        fs::write(root.join("c.bin"), b"needle\0binary").unwrap();

        let batches: Vec<_> = block_on(search_folder(root.clone(), regex_for(&query("needle"))).collect());
        let files: Vec<SearchFile> = batches.into_iter().flatten().collect();
        assert_eq!(files.len(), 1);
        assert!(files[0].path.ends_with("src/a.txt"));
        assert_eq!(files[0].hits.iter().map(|h| h.line).collect::<Vec<_>>(), [1, 2]);
        assert_eq!(files[0].hits[1].before, "and ");
        fs::remove_dir_all(root).unwrap();
    }
}
