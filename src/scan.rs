//! Reads a whole folder's comments in the background, and applies folder-wide cleanups.

use crate::cleanup::{self, Change};
use crate::files;
use crate::highlight::{Highlighter, Hunk};
use crate::rules::{Decision, Rules};
use iced::futures::Stream;
use iced::futures::channel::mpsc;
use ignore::WalkBuilder;
use std::collections::{HashMap, HashSet};
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

const MAX_FILE_SIZE: u64 = 2 * 1024 * 1024;
const MAX_FILES: usize = 50_000;
const BATCH_INTERVAL: Duration = Duration::from_millis(150);
const NEVER_SCANNED: [&str; 2] = [".git", "node_modules"];

#[derive(Clone, Debug)]
pub struct ScannedFile {
    pub path: PathBuf,
    pub hash: u64,
    pub language: String,
    /// Shared, so testing rules in Settings doesn't copy every comment per keystroke.
    pub hunks: Arc<Vec<Hunk>>,
    /// One per hunk.
    pub decisions: Vec<Decision>,
}

pub fn candidates(root: &Path) -> Vec<PathBuf> {
    let mut builder = WalkBuilder::new(root);
    builder
        .hidden(false)
        .require_git(false)
        .git_global(false)
        .ignore(false)
        .max_filesize(Some(MAX_FILE_SIZE))
        .filter_entry(|entry| {
            !entry.file_name().to_str().is_some_and(|name| NEVER_SCANNED.contains(&name))
        });
    builder
        .build()
        .flatten()
        .filter(|entry| entry.file_type().is_some_and(|kind| kind.is_file()))
        .map(|entry| entry.into_path())
        .take(MAX_FILES)
        .collect()
}

fn scan_file(highlighter: &Highlighter, rules: &Rules, path: &Path, theme: &str) -> Option<ScannedFile> {
    let language = highlighter.language_for(path);
    if language == "plain" {
        return None;
    }
    let bytes = std::fs::read(path).ok()?;
    let decoded = files::decode(&bytes);
    if decoded.read_only.is_some() {
        return None;
    }
    let doc = catch_unwind(AssertUnwindSafe(|| highlighter.highlight(&decoded.body, &language, theme)))
        .ok()?
        .ok()?;
    let syntax = highlighter.syntax_for(&language);
    let decisions = doc.hunks.iter().map(|hunk| rules.decide(hunk, syntax)).collect();
    Some(ScannedFile {
        path: path.to_path_buf(),
        hash: files::hash_of(&bytes),
        language,
        hunks: Arc::new(doc.hunks),
        decisions,
    })
}

/// Scans a folder on worker threads, yielding results in timed batches. Dropping the stream stops the workers.
pub fn scan_folder(
    highlighter: Arc<Highlighter>,
    rules: Arc<Rules>,
    root: PathBuf,
    theme: String,
) -> impl Stream<Item = Vec<ScannedFile>> {
    let (sender, receiver) = mpsc::unbounded();
    std::thread::spawn(move || {
        let paths = candidates(&root);
        let next = AtomicUsize::new(0);
        let batch = Mutex::new((Vec::new(), Instant::now()));
        let workers = std::thread::available_parallelism().map_or(2, |n| n.get() / 2).clamp(1, 8);
        std::thread::scope(|scope| {
            for _ in 0..workers {
                scope.spawn(|| {
                    while let Some(path) = paths.get(next.fetch_add(1, Ordering::Relaxed)) {
                        if sender.is_closed() {
                            break;
                        }
                        let Some(file) = scan_file(&highlighter, &rules, path, &theme) else {
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

/// Re-reads files that changed on disk: what to update, and what to forget.
pub fn rescan(
    highlighter: &Highlighter,
    rules: &Rules,
    root: &Path,
    paths: &[PathBuf],
    theme: &str,
) -> (Vec<ScannedFile>, Vec<PathBuf>) {
    let mut updated = Vec::new();
    let mut removed = Vec::new();
    for path in paths {
        let skipped = path
            .components()
            .any(|part| NEVER_SCANNED.iter().any(|name| part.as_os_str() == *name));
        if skipped || path.is_dir() || !path.starts_with(root) {
            continue;
        }
        let small_enough = std::fs::metadata(path).is_ok_and(|meta| meta.len() <= MAX_FILE_SIZE);
        let file = (small_enough && !files::is_path_ignored(root, path))
            .then(|| scan_file(highlighter, rules, path, theme))
            .flatten();
        match file {
            Some(file) => updated.push(file),
            None => removed.push(path.clone()),
        }
    }
    (updated, removed)
}

const MAX_DIFFERENCES: usize = 60;
const PREVIEW_CHARS: usize = 100;

pub struct ImpactFile {
    pub path: PathBuf,
    pub label: String,
    pub language: String,
    pub hunks: Arc<Vec<Hunk>>,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Tally {
    pub delete: usize,
    pub squash: usize,
    pub protect: usize,
    pub keep: usize,
}

impl Tally {
    fn add(&mut self, decision: &Decision) {
        match decision {
            Decision::Delete(_) => self.delete += 1,
            Decision::Squash(_) => self.squash += 1,
            Decision::Protected(_) => self.protect += 1,
            Decision::Keep => self.keep += 1,
        }
    }
}

#[derive(Clone, Debug)]
pub struct Difference {
    pub label: String,
    pub line: usize,
    pub comment: String,
    pub before: String,
    pub after: String,
}

#[derive(Clone, Debug, Default)]
pub struct RuleImpact {
    pub files: usize,
    pub now: Tally,
    pub saved: Tally,
    pub differences: Vec<Difference>,
    pub more: usize,
}

/// Compares the edited rules with the saved ones. Gives up, returning `None`, as soon as `still_wanted` says a newer edit has made this out of date.
pub fn rule_impact(
    highlighter: &Highlighter,
    files: &[ImpactFile],
    saved: &Rules,
    edited: &Rules,
    ignored: &HashMap<PathBuf, HashSet<u64>>,
    still_wanted: impl Fn() -> bool,
) -> Option<RuleImpact> {
    let mut impact = RuleImpact::default();
    for file in files {
        if !still_wanted() {
            return None;
        }
        let skipped = ignored.get(&file.path);
        let syntax = highlighter.syntax_for(&file.language);
        let mut affected = false;
        for hunk in file.hunks.iter().filter(|hunk| !skipped.is_some_and(|keys| keys.contains(&hunk.key))) {
            let before = saved.decide(hunk, syntax);
            let after = edited.decide(hunk, syntax);
            impact.saved.add(&before);
            impact.now.add(&after);
            affected |= after.is_fix();
            if before == after {
                continue;
            }
            if impact.differences.len() == MAX_DIFFERENCES {
                impact.more += 1;
                continue;
            }
            let first_line = hunk.raw.lines().map(str::trim).find(|line| !line.is_empty()).unwrap_or("");
            impact.differences.push(Difference {
                label: file.label.clone(),
                line: hunk.first + 1,
                comment: first_line.chars().take(PREVIEW_CHARS).collect(),
                before: before.summary(),
                after: after.summary(),
            });
        }
        impact.files += usize::from(affected);
    }
    Some(impact)
}

pub struct PlanItem {
    pub hunk: Hunk,
    pub change: Change,
    pub reason: String,
    pub include: bool,
}

impl PlanItem {
    pub fn is_squash(&self) -> bool {
        matches!(self.change, Change::Replace(_))
    }
}

pub struct PlanFile {
    pub path: PathBuf,
    pub label: String,
    pub hash: u64,
    pub items: Vec<PlanItem>,
    pub expanded: bool,
}

/// Proposed changes, waiting on the preview page for the user's say-so.
pub struct Plan {
    pub folder: bool,
    pub files: Vec<PlanFile>,
}

impl Plan {
    /// (ticked, total) among deletes or squashes.
    pub fn count(&self, squash: bool) -> (usize, usize) {
        let mut ticked = 0;
        let mut total = 0;
        for item in self.files.iter().flat_map(|file| &file.items) {
            if item.is_squash() == squash {
                total += 1;
                ticked += usize::from(item.include);
            }
        }
        (ticked, total)
    }
}

pub struct PlanFileChanges {
    pub path: PathBuf,
    pub hash: u64,
    pub changes: Vec<(Hunk, Change)>,
}

/// What a folder cleanup needs to restore each file it wrote.
#[derive(Clone, Debug)]
pub struct FolderUndo {
    pub path: PathBuf,
    pub previous: Arc<Vec<u8>>,
    pub written_hash: u64,
}

#[derive(Clone, Debug, Default)]
pub struct PlanOutcome {
    pub undo: Vec<FolderUndo>,
    pub comments: usize,
    pub skipped: Vec<String>,
}

/// Applies each file independently: one that changed since the preview is skipped.
pub fn apply_plan(highlighter: &Highlighter, theme: &str, files_to_change: Vec<PlanFileChanges>) -> PlanOutcome {
    let mut outcome = PlanOutcome::default();
    for file in files_to_change {
        let name = file.path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
        let result = files::load(highlighter, &file.path, theme, None)
            .and_then(|loaded| loaded.ok_or_else(|| "couldn't read it".to_owned()))
            .and_then(|loaded| {
                if loaded.hash != file.hash {
                    return Err("it changed since the preview".to_owned());
                }
                cleanup::apply_changes(highlighter, &file.path, theme, loaded.hash, &loaded.doc, &file.changes)
            });
        match result {
            Ok(applied) => {
                outcome.comments += applied.changed;
                outcome.undo.push(FolderUndo {
                    path: file.path,
                    previous: applied.previous,
                    written_hash: applied.loaded.hash,
                });
            }
            Err(error) => outcome.skipped.push(format!("{name}: {error}")),
        }
    }
    outcome
}

/// (files restored, files left alone because they've changed since).
pub fn undo_plan(highlighter: &Highlighter, theme: &str, undo: &[FolderUndo]) -> (usize, usize) {
    let restored = undo
        .iter()
        .filter(|file| {
            cleanup::undo(highlighter, &file.path, theme, file.written_hash, &file.previous).is_ok()
        })
        .count();
    (restored, undo.len() - restored)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    #[test]
    fn scanning_follows_gitignore_and_skips_dependencies() {
        let root = std::env::temp_dir().join(format!("commentcleanup-scan-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(root.join("src")).unwrap();
        fs::create_dir_all(root.join("build")).unwrap();
        fs::create_dir_all(root.join("node_modules/pkg")).unwrap();
        fs::write(root.join(".gitignore"), "build/\n").unwrap();
        fs::write(root.join("src/a.ts"), "// a").unwrap();
        fs::write(root.join("build/b.ts"), "// b").unwrap();
        fs::write(root.join("node_modules/pkg/c.ts"), "// c").unwrap();

        let found = candidates(&root);
        assert!(found.iter().any(|p| p.ends_with("a.ts")));
        assert!(!found.iter().any(|p| p.ends_with("b.ts")));
        assert!(!found.iter().any(|p| p.ends_with("c.ts")));
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn testing_rules_counts_both_sets_and_lists_only_what_differs() {
        use crate::highlight::tests::highlighter;
        use crate::rules::Settings;
        let h = highlighter();
        let doc = h.highlight("// keep me\nlet a;\n// wip one\nlet b;\n// wip two\n", "javascript", "Dark+").unwrap();
        let ignored_key = doc.hunks[2].key;
        let files = [ImpactFile {
            path: "a.js".into(),
            label: "a.js".into(),
            language: "javascript".into(),
            hunks: Arc::new(doc.hunks),
        }];
        let saved = Rules::new(&Settings::default());
        let edited = Rules::new(&Settings { delete_patterns: vec!["^wip".to_owned()], ..Settings::default() });
        let ignored = HashMap::from([(PathBuf::from("a.js"), HashSet::from([ignored_key]))]);

        let impact = rule_impact(h, &files, &saved, &edited, &ignored, || true).unwrap();
        assert_eq!(impact.files, 1);
        assert_eq!(impact.saved, Tally { keep: 2, ..Tally::default() });
        assert_eq!(impact.now, Tally { delete: 1, keep: 1, ..Tally::default() });
        assert_eq!(impact.differences.len(), 1);
        assert_eq!(impact.differences[0].line, 3);
        assert_eq!(impact.differences[0].comment, "// wip one");
        assert_eq!(impact.differences[0].after, "Deleted: your delete pattern");

        assert!(rule_impact(h, &files, &saved, &edited, &ignored, || false).is_none());
    }

    #[test]
    fn a_plan_counts_ticked_and_total_separately() {
        let hunk = Hunk {
            first: 0,
            last: 0,
            start: 0,
            end: 1,
            kind: crate::highlight::HunkKind::LineRun,
            whole_line: true,
            raw: String::new(),
            key: 0,
        };
        let item = |change, include| PlanItem { hunk: hunk.clone(), change, reason: String::new(), include };
        let plan = Plan {
            folder: true,
            files: vec![PlanFile {
                path: PathBuf::new(),
                label: String::new(),
                hash: 0,
                expanded: false,
                items: vec![
                    item(Change::Delete, true),
                    item(Change::Delete, false),
                    item(Change::Replace("x".into()), false),
                ],
            }],
        };
        assert_eq!(plan.count(false), (1, 2));
        assert_eq!(plan.count(true), (0, 1));
    }
}
