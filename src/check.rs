//! `commentcleanup-check`: reports which rules would modify comments, for hooks and CI.
//
//! Returns exit code 0 if nothing would change, 1 if a comment would be changed, and 2 if the check failed; it never writes any files.

use crate::git;
use crate::highlight::Highlighter;
use crate::rules::{self, Decision, Rules};
use crate::scan::{self, ScannedFile};
use crate::session;
use crate::vscode;
use iced::futures::StreamExt;
use iced::futures::executor::block_on;
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::Arc;

const CLEAN: i32 = 0;
const FOUND: i32 = 1;
const BROKEN: i32 = 2;

const HELP: &str = "\
Reports comments the CommentCleanup rules would change. Never edits a file.

Usage: commentcleanup-check [FOLDER] [OPTIONS]

FOLDER defaults to the current folder.

Options:
  --changed     Only comments on lines changed since the last commit
  --no-squash   Don't count comments that would only be squashed
  --editors     Also use grammars from installed editors (results can then
                differ between machines)
  -h, --help    Show this

Rules come from the folder's .commentcleanup.json, then the app's settings,
then the defaults.

Exit status: 0 nothing to change, 1 something would change, 2 couldn't check.
";

#[derive(Debug, Default, PartialEq, Eq)]
struct Options {
    folder: Option<PathBuf>,
    changed: bool,
    no_squash: bool,
    editors: bool,
    help: bool,
}

fn parse_options(arguments: &[String]) -> Result<Options, String> {
    let mut options = Options::default();
    for argument in arguments {
        match argument.as_str() {
            "--changed" => options.changed = true,
            "--no-squash" => options.no_squash = true,
            "--editors" => options.editors = true,
            "-h" | "--help" => options.help = true,
            other if other.starts_with('-') => return Err(format!("Unknown option: {other}")),
            _ if options.folder.is_some() => return Err("Give only one folder.".to_owned()),
            folder => options.folder = Some(folder.into()),
        }
    }
    Ok(options)
}

#[derive(Debug, PartialEq, Eq)]
struct Finding {
    file: String,
    line: usize,
    action: String,
    comment: String,
}

fn action(decision: &Decision, no_squash: bool) -> Option<String> {
    match decision {
        Decision::Delete(why) => Some(format!("delete ({why})")),
        Decision::Squash(_) if !no_squash => Some("squash".to_owned()),
        _ => None,
    }
}

/// `changed` limits it to those lines, and a file missing from it has none.
fn findings(
    files: &[ScannedFile],
    root: &Path,
    ignored: &HashMap<PathBuf, HashSet<u64>>,
    changed: Option<&git::Repo>,
    no_squash: bool,
) -> Vec<Finding> {
    let mut found = Vec::new();
    for file in files {
        let skipped = ignored.get(&file.path);
        let lines = changed.map(|repo| repo.files.get(&file.path));
        let name = file.path.strip_prefix(root).unwrap_or(&file.path).to_string_lossy().replace('\\', "/");
        for (hunk, decision) in file.hunks.iter().zip(&file.decisions) {
            let wanted = lines.is_none_or(|lines| lines.is_some_and(|f| f.changed.touches(hunk.first, hunk.last)));
            if !wanted || skipped.is_some_and(|keys| keys.contains(&hunk.key)) {
                continue;
            }
            if let Some(action) = action(decision, no_squash) {
                found.push(Finding {
                    file: name.clone(),
                    line: hunk.first + 1,
                    action,
                    comment: hunk.raw.lines().next().unwrap_or("").trim().chars().take(80).collect(),
                });
            }
        }
    }
    found.sort_by(|a, b| (&a.file, a.line).cmp(&(&b.file, b.line)));
    found
}

fn load_rules(root: &Path) -> Result<Rules, String> {
    let settings = rules::read_settings(&rules::workspace_settings_path(root))
        .or_else(|| rules::app_settings_path().and_then(|path| rules::read_settings(&path)))
        .unwrap_or_default();
    let compiled = Rules::new(&settings);
    if compiled.errors.is_empty() {
        Ok(compiled)
    } else {
        Err(format!("Not a valid pattern: {}", compiled.errors.join(", ")))
    }
}

fn check(options: &Options) -> Result<i32, String> {
    let folder = options.folder.clone().unwrap_or_else(|| ".".into());
    let root = std::path::absolute(&folder).map_err(|e| e.to_string())?;
    if !root.is_dir() {
        return Err(format!("Not a folder: {}", folder.display()));
    }
    let repo = if options.changed {
        Some(git::read(&root).ok_or("--changed needs Git and a repository.")?)
    } else {
        None
    };
    let compiled = Arc::new(load_rules(&root)?);
    let (highlighter, _) = Highlighter::load(vscode::scan(options.editors))?;
    let theme = highlighter.theme_names.first().cloned().ok_or("No themes are available.")?;
    let batches: Vec<Vec<ScannedFile>> =
        block_on(scan::scan_folder(Arc::new(highlighter), compiled, root.clone(), theme).collect());
    let files: Vec<ScannedFile> = batches.into_iter().flatten().collect();

    let ignored = session::load().ignored;
    let found = findings(&files, &root, &ignored, repo.as_ref(), options.no_squash);
    for finding in &found {
        println!("{}:{}: {}: {}", finding.file, finding.line, finding.action, finding.comment);
    }
    if found.is_empty() {
        println!("Checked {} files. Nothing to change.", files.len());
        return Ok(CLEAN);
    }
    let touched: HashSet<&str> = found.iter().map(|finding| finding.file.as_str()).collect();
    let plural = |count: usize, word: &str| format!("{count} {word}{}", if count == 1 { "" } else { "s" });
    println!("{} in {} would change.", plural(found.len(), "comment"), plural(touched.len(), "file"));
    Ok(FOUND)
}

/// Runs the check and gives the process's exit status.
pub fn run(arguments: &[String]) -> i32 {
    std::panic::set_hook(Box::new(|_| {}));
    let options = match parse_options(arguments) {
        Ok(options) => options,
        Err(error) => {
            eprintln!("{error}\n\n{HELP}");
            return BROKEN;
        }
    };
    if options.help {
        print!("{HELP}");
        return CLEAN;
    }
    check(&options).unwrap_or_else(|error| {
        eprintln!("{error}");
        BROKEN
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::git::{Changed, FileState, GitFile, Repo};
    use crate::highlight::tests::highlighter;
    use crate::rules::Settings;

    fn scanned(path: &str, source: &str) -> ScannedFile {
        let h = highlighter();
        let doc = h.highlight(source, "javascript", "Dark+").unwrap();
        let rules = Rules::new(&Settings::default());
        let syntax = h.syntax_for("javascript");
        ScannedFile {
            path: path.into(),
            hash: 0,
            language: "javascript".into(),
            decisions: doc.hunks.iter().map(|hunk| rules.decide(hunk, syntax)).collect(),
            hunks: Arc::new(doc.hunks),
        }
    }

    const SOURCE: &str = "// -----\nlet a;\n// one\n// two\nlet b;\n// fine\n";

    #[test]
    fn options_are_read_and_mistakes_refused() {
        let parse = |list: &[&str]| parse_options(&list.iter().map(|s| s.to_string()).collect::<Vec<_>>());
        let options = parse(&["src", "--changed", "--no-squash"]).unwrap();
        assert_eq!(options, Options { folder: Some("src".into()), changed: true, no_squash: true, ..Options::default() });
        assert!(parse(&["--nope"]).is_err());
        assert!(parse(&["a", "b"]).is_err());
    }

    #[test]
    fn findings_cover_deletes_and_squashes_in_order() {
        let found = findings(&[scanned("C:/p/a.js", SOURCE)], Path::new("C:/p"), &HashMap::new(), None, false);
        assert_eq!(found.iter().map(|f| (f.line, f.action.as_str())).collect::<Vec<_>>(), [(1, "delete (divider)"), (3, "squash")]);
        assert_eq!(found[0].file, "a.js");
    }

    #[test]
    fn squashes_can_be_left_out() {
        let found = findings(&[scanned("C:/p/a.js", SOURCE)], Path::new("C:/p"), &HashMap::new(), None, true);
        assert_eq!(found.len(), 1);
    }

    #[test]
    fn ignored_comments_are_left_out() {
        let file = scanned("C:/p/a.js", SOURCE);
        let ignored = HashMap::from([(file.path.clone(), HashSet::from([file.hunks[0].key]))]);
        let found = findings(&[file], Path::new("C:/p"), &ignored, None, false);
        assert_eq!(found.iter().map(|f| f.line).collect::<Vec<_>>(), [3]);
    }

    #[test]
    fn only_changed_lines_count_when_asked() {
        let file = scanned("C:/p/a.js", SOURCE);
        let other = scanned("C:/p/untouched.js", SOURCE);
        let repo = Repo {
            branch: "main".into(),
            files: HashMap::from([(
                file.path.clone(),
                GitFile { state: FileState::Modified, changed: Changed::Ranges(vec![(3, 3)]) },
            )]),
        };
        let found = findings(&[file, other], Path::new("C:/p"), &HashMap::new(), Some(&repo), false);
        assert_eq!(found.iter().map(|f| (f.file.as_str(), f.line)).collect::<Vec<_>>(), [("a.js", 3)]);
    }
}
