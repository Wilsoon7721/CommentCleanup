//! What Git knows about the folder

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::process::Command;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FileState {
    Modified,
    Added,
    Untracked,
    Conflict,
}

impl FileState {
    pub fn letter(self) -> &'static str {
        match self {
            FileState::Modified => "M",
            FileState::Added => "A",
            FileState::Untracked => "U",
            FileState::Conflict => "!",
        }
    }

    pub fn describe(self) -> &'static str {
        match self {
            FileState::Modified => "Uncommitted changes",
            FileState::Added => "New file, not committed",
            FileState::Untracked => "Not tracked by Git",
            FileState::Conflict => "Merge conflict",
        }
    }
}

/// Which lines differ from the last commit, numbered from zero.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Changed {
    All,
    /// Sorted `(first, last)`, both included.
    Ranges(Vec<(usize, usize)>),
}

impl Changed {
    pub fn touches(&self, first: usize, last: usize) -> bool {
        match self {
            Changed::All => true,
            Changed::Ranges(ranges) => {
                let after = ranges.partition_point(|&(_, end)| end < first);
                ranges.get(after).is_some_and(|&(start, _)| start <= last)
            }
        }
    }

    pub fn line_count(&self) -> Option<usize> {
        match self {
            Changed::All => None,
            Changed::Ranges(ranges) => Some(ranges.iter().map(|(a, b)| b - a + 1).sum()),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GitFile {
    pub state: FileState,
    pub changed: Changed,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Repo {
    pub branch: String,
    pub files: HashMap<PathBuf, GitFile>,
}

fn git(root: &Path, arguments: &[&str]) -> Option<Vec<u8>> {
    let mut command = Command::new("git");
    command
        // Keeps `status` from rewriting the index, which the folder watcher would report, which would run `status` again.
        .env("GIT_OPTIONAL_LOCKS", "0")
        .arg("-C")
        .arg(root)
        .args(["-c", "core.quotepath=off", "-c", "core.fsmonitor=false"])
        .args(arguments);
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        command.creation_flags(CREATE_NO_WINDOW);
    }
    let output = command.output().ok()?;
    output.status.success().then_some(output.stdout)
}

/// `None` when Git isn't installed or the folder isn't in a repository.
pub fn read(root: &Path) -> Option<Repo> {
    let prefix = String::from_utf8(git(root, &["rev-parse", "--show-prefix"])?).ok()?;
    let prefix = prefix.trim_end_matches(['\r', '\n']).to_owned();
    let status = git(root, &["status", "--porcelain=v1", "-z", "--branch", "--untracked-files=all"])?;
    let (branch, states) = parse_status(&String::from_utf8_lossy(&status));
    let diff = git(
        root,
        &["diff", "HEAD", "--unified=0", "--no-color", "--no-ext-diff", "--no-textconv", "--no-relative"],
    );
    let mut ranges = diff.map_or_else(HashMap::new, |diff| parse_diff(&String::from_utf8_lossy(&diff)));
    let files = states
        .into_iter()
        .filter_map(|(path, state)| {
            let changed = match state {
                FileState::Untracked => Changed::All,
                _ => ranges.remove(&path).map_or(Changed::All, Changed::Ranges),
            };
            let relative = path.strip_prefix(prefix.as_str())?;
            Some((root.join(relative), GitFile { state, changed }))
        })
        .collect();
    Some(Repo { branch, files })
}

pub fn read_staged(root: &Path) -> Option<Repo> {
    let prefix = String::from_utf8(git(root, &["rev-parse", "--show-prefix"])?).ok()?;
    let prefix = prefix.trim_end_matches(['\r', '\n']).to_owned();
    let diff = git(
        root,
        &["diff", "--cached", "--unified=0", "--no-color", "--no-ext-diff", "--no-textconv", "--no-relative"],
    )?;
    let files = parse_diff(&String::from_utf8_lossy(&diff))
        .into_iter()
        .filter_map(|(path, ranges)| {
            let relative = path.strip_prefix(prefix.as_str())?;
            Some((root.join(relative), GitFile { state: FileState::Modified, changed: Changed::Ranges(ranges) }))
        })
        .collect();
    Some(Repo { branch: String::new(), files })
}

pub fn pre_commit_path(root: &Path) -> Option<PathBuf> {
    let path = String::from_utf8(git(root, &["rev-parse", "--git-path", "hooks/pre-commit"])?).ok()?;
    Some(root.join(path.trim_end_matches(['\r', '\n'])))
}

/// The branch, and each file's state. Paths are relative to the repository.
fn parse_status(output: &str) -> (String, Vec<(String, FileState)>) {
    let mut branch = String::new();
    let mut files = Vec::new();
    let mut fields = output.split('\0');
    while let Some(field) = fields.next() {
        if let Some(header) = field.strip_prefix("## ") {
            branch = branch_name(header);
            continue;
        }
        if field.len() < 4 {
            continue;
        }
        let (x, y) = (field.as_bytes()[0], field.as_bytes()[1]);
        let path = &field[3..];
        // A rename is followed by the name it used to have.
        if matches!(x, b'R' | b'C') || matches!(y, b'R' | b'C') {
            fields.next();
        }
        let conflict = x == b'U' || y == b'U' || (x == y && matches!(x, b'A' | b'D'));
        let state = if x == b'?' {
            FileState::Untracked
        } else if conflict {
            FileState::Conflict
        } else if x == b'D' || y == b'D' {
            continue;
        } else if x == b'A' {
            FileState::Added
        } else {
            FileState::Modified
        };
        files.push((path.to_owned(), state));
    }
    (branch, files)
}

fn branch_name(header: &str) -> String {
    if let Some(unborn) = header.strip_prefix("No commits yet on ") {
        return unborn.to_owned();
    }
    if header.starts_with("HEAD (no branch)") {
        return "detached HEAD".to_owned();
    }
    header.split("...").next().unwrap_or(header).trim().to_owned()
}

/// Lines on the new side of a zero-context diff, per file.
fn parse_diff(output: &str) -> HashMap<String, Vec<(usize, usize)>> {
    let mut files: HashMap<String, Vec<(usize, usize)>> = HashMap::new();
    let mut current: Option<String> = None;
    for line in output.lines() {
        if let Some(name) = line.strip_prefix("+++ ") {
            current = diff_path(name);
            if let Some(path) = &current {
                files.entry(path.clone()).or_default();
            }
        } else if line.starts_with("diff --git ") {
            current = None;
        } else if line.starts_with("@@ ")
            && let (Some(path), Some(range)) = (&current, new_side(line))
            && let Some(ranges) = files.get_mut(path)
        {
            ranges.push(range);
        }
    }
    files
}

fn diff_path(name: &str) -> Option<String> {
    let name = name.trim_end_matches(['\t', '\r']);
    let name = match name.strip_prefix('"').and_then(|n| n.strip_suffix('"')) {
        Some(quoted) => unquote(quoted),
        None => name.to_owned(),
    };
    name.strip_prefix("b/").map(str::to_owned)
}

fn unquote(quoted: &str) -> String {
    let mut out = String::new();
    let mut chars = quoted.chars();
    while let Some(c) = chars.next() {
        if c != '\\' {
            out.push(c);
            continue;
        }
        match chars.next() {
            Some('t') => out.push('\t'),
            Some('n') => out.push('\n'),
            Some(other) => out.push(other),
            None => {}
        }
    }
    out
}

fn new_side(header: &str) -> Option<(usize, usize)> {
    let new = header.split(' ').find(|part| part.starts_with('+'))?;
    let (start, count) = match new[1..].split_once(',') {
        Some((start, count)) => (start.parse::<usize>().ok()?, count.parse::<usize>().ok()?),
        None => (new[1..].parse::<usize>().ok()?, 1),
    };
    (count > 0 && start > 0).then(|| (start - 1, start + count - 2))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    #[test]
    fn status_covers_each_state_and_skips_a_renames_old_name() {
        let output = "## main...origin/main [ahead 1]\0 M src/a.rs\0A  new.rs\0?? notes.txt\0UU clash.rs\0 D gone.rs\0R  moved.rs\0old.rs\0";
        let (branch, files) = parse_status(output);
        assert_eq!(branch, "main");
        assert_eq!(
            files,
            [
                ("src/a.rs".to_owned(), FileState::Modified),
                ("new.rs".to_owned(), FileState::Added),
                ("notes.txt".to_owned(), FileState::Untracked),
                ("clash.rs".to_owned(), FileState::Conflict),
                ("moved.rs".to_owned(), FileState::Modified),
            ]
        );
    }

    #[test]
    fn branch_names_cover_new_and_detached_repositories() {
        assert_eq!(branch_name("No commits yet on main"), "main");
        assert_eq!(branch_name("HEAD (no branch)"), "detached HEAD");
        assert_eq!(branch_name("feature/x"), "feature/x");
    }

    #[test]
    fn a_diff_gives_the_new_lines_and_ignores_pure_deletions() {
        let output = "diff --git a/a.rs b/a.rs\n--- a/a.rs\n+++ b/a.rs\n@@ -1,2 +1 @@ fn x()\n-a\n-b\n+c\n@@ -9,2 +8,0 @@\n-d\n-e\n@@ -20 +19,3 @@\n-f\n+g\n+h\n+i\ndiff --git \"a/b c.rs\" \"b/b c.rs\"\n--- \"a/b c.rs\"\n+++ \"b/b c.rs\"\n@@ -1 +1 @@\n-x\n+y\n";
        let files = parse_diff(output);
        assert_eq!(files["a.rs"], [(0, 0), (18, 20)]);
        assert_eq!(files["b c.rs"], [(0, 0)]);
    }

    #[test]
    fn a_comment_counts_as_changed_when_any_of_its_lines_are() {
        let changed = Changed::Ranges(vec![(4, 6), (10, 10)]);
        assert!(changed.touches(6, 8));
        assert!(changed.touches(2, 4));
        assert!(changed.touches(10, 10));
        assert!(!changed.touches(7, 9));
        assert!(!changed.touches(11, 20));
        assert!(!changed.touches(0, 3));
        assert_eq!(changed.line_count(), Some(4));
        assert!(Changed::All.touches(100, 200));
    }

    fn run(root: &Path, arguments: &[&str]) {
        assert!(git(root, arguments).is_some(), "git {arguments:?}");
    }

    #[test]
    fn a_real_repository_reports_changes_relative_to_the_opened_folder() {
        let base = std::env::temp_dir().join(format!("commentcleanup-git-{}", std::process::id()));
        let _ = fs::remove_dir_all(&base);
        let folder = base.join("sub");
        fs::create_dir_all(&folder).unwrap();
        assert!(read(&folder).is_none(), "not a repository yet");
        run(&base, &["init", "-q"]);
        run(&base, &["config", "user.email", "test@example.com"]);
        run(&base, &["config", "user.name", "Test"]);
        run(&base, &["config", "core.autocrlf", "false"]);
        fs::write(folder.join("a.txt"), "one\ntwo\nthree\nfour\n").unwrap();
        fs::write(folder.join("clean.txt"), "same\n").unwrap();
        fs::write(base.join("outside.txt"), "x\n").unwrap();
        run(&base, &["add", "."]);
        run(&base, &["commit", "-q", "-m", "first"]);

        fs::write(folder.join("a.txt"), "one\ntwo\nTHREE\nfour\nfive\n").unwrap();
        fs::write(folder.join("new.txt"), "brand new\n").unwrap();
        fs::write(base.join("outside.txt"), "y\n").unwrap();

        let repo = read(&folder).unwrap();
        assert!(!repo.branch.is_empty());
        assert_eq!(repo.files.len(), 2, "{:?}", repo.files);
        assert_eq!(
            repo.files[&folder.join("a.txt")],
            GitFile { state: FileState::Modified, changed: Changed::Ranges(vec![(2, 2), (4, 4)]) }
        );
        assert_eq!(
            repo.files[&folder.join("new.txt")],
            GitFile { state: FileState::Untracked, changed: Changed::All }
        );
        fs::remove_dir_all(base).unwrap();
    }
}
