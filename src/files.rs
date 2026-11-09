use crate::highlight::{Document, Highlighter};
use ignore::gitignore::{Gitignore, GitignoreBuilder};
use std::collections::HashSet;
use std::collections::hash_map::DefaultHasher;
use std::fs;
use std::hash::{Hash, Hasher};
use std::path::{Path, PathBuf};
use std::sync::Arc;

pub const TEMP_SUFFIX: &str = ".commentcleanup.tmp";
const BOM: &str = "\u{feff}";

#[derive(Clone, Debug)]
pub struct Entry {
    pub path: PathBuf,
    pub name: String,
    pub is_dir: bool,
    pub depth: u16,
    /// Matched by a `.gitignore`, directly or through a parent folder.
    pub ignored: bool,
}

/// Lists the root and every expanded folder, directories first.
pub fn list_tree(root: &Path, expanded: &HashSet<PathBuf>) -> Vec<Entry> {
    let mut entries = Vec::new();
    let mut rules = rules_above(root);
    walk(root, 0, expanded, &mut rules, false, &mut entries);
    entries
}

fn read_ignore_file(base: &Path, file: &Path) -> Option<Gitignore> {
    if !file.is_file() {
        return None;
    }
    let mut builder = GitignoreBuilder::new(base);
    let _ = builder.add(file);
    builder.build().ok()
}

/// Rules that apply from outside the opened folder, when it sits inside a repository.
fn rules_above(root: &Path) -> Vec<Gitignore> {
    let Some(repo) = root.ancestors().find(|dir| dir.join(".git").exists()) else {
        return Vec::new();
    };
    let mut rules: Vec<Gitignore> =
        read_ignore_file(repo, &repo.join(".git/info/exclude")).into_iter().collect();
    let between = root.strip_prefix(repo).unwrap_or(Path::new(""));
    let mut dir = repo.to_path_buf();
    for part in between.components() {
        rules.extend(read_ignore_file(&dir, &dir.join(".gitignore")));
        dir.push(part);
    }
    rules
}

/// The deepest file with an opinion decides, and a later line beats an earlier one.
fn is_ignored(rules: &[Gitignore], path: &Path, is_dir: bool) -> bool {
    for file in rules.iter().rev() {
        let verdict = file.matched(path, is_dir);
        if verdict.is_ignore() {
            return true;
        }
        if verdict.is_whitelist() {
            return false;
        }
    }
    false
}

/// Whether `.gitignore` files exclude one path, for checking a single changed file.
pub fn is_path_ignored(root: &Path, path: &Path) -> bool {
    let Ok(relative) = path.strip_prefix(root) else {
        return true;
    };
    let mut rules = rules_above(root);
    let mut current = root.to_path_buf();
    let parts: Vec<_> = relative.components().collect();
    for (index, part) in parts.iter().enumerate() {
        rules.extend(read_ignore_file(&current, &current.join(".gitignore")));
        current.push(part);
        if is_ignored(&rules, &current, index + 1 < parts.len()) {
            return true;
        }
    }
    false
}

fn walk(
    dir: &Path,
    depth: u16,
    expanded: &HashSet<PathBuf>,
    rules: &mut Vec<Gitignore>,
    parent_ignored: bool,
    out: &mut Vec<Entry>,
) {
    let Ok(read) = fs::read_dir(dir) else {
        return;
    };
    let own_rules = read_ignore_file(dir, &dir.join(".gitignore"));
    let added = own_rules.is_some();
    rules.extend(own_rules);

    let mut children: Vec<Entry> = read
        .flatten()
        .filter(|e| e.file_name() != ".git")
        .map(|e| {
            let is_dir = e.file_type().is_ok_and(|t| t.is_dir());
            let path = e.path();
            Entry {
                ignored: parent_ignored || is_ignored(rules, &path, is_dir),
                name: e.file_name().to_string_lossy().into_owned(),
                path,
                is_dir,
                depth,
            }
        })
        .collect();
    children.sort_by_key(|e| (!e.is_dir, e.name.to_lowercase()));
    for child in children {
        let path = child.path.clone();
        let ignored = child.ignored;
        let recurse = child.is_dir && expanded.contains(&path);
        out.push(child);
        if recurse {
            walk(&path, depth + 1, expanded, rules, ignored, out);
        }
    }
    if added {
        rules.pop();
    }
}

pub fn hash_of(bytes: &[u8]) -> u64 {
    let mut hasher = DefaultHasher::new();
    bytes.hash(&mut hasher);
    hasher.finish()
}

/// A file's text, plus whether it is safe to write back.
pub struct Decoded {
    pub body: String,
    pub bom: bool,
    /// Why the file cannot be edited, if it can't.
    pub read_only: Option<String>,
}

pub fn decode(bytes: &[u8]) -> Decoded {
    if bytes.iter().take(8192).any(|b| *b == 0) {
        return Decoded {
            body: "(binary file, or text in an encoding that isn't UTF-8)\n".into(),
            bom: false,
            read_only: Some("This file isn't UTF-8 text.".into()),
        };
    }
    match std::str::from_utf8(bytes) {
        Ok(text) => {
            let body = text.strip_prefix(BOM);
            Decoded {
                bom: body.is_some(),
                body: body.unwrap_or(text).to_owned(),
                read_only: None,
            }
        }
        // Writing lossy text back would corrupt the file.
        Err(_) => Decoded {
            body: String::from_utf8_lossy(bytes).into_owned(),
            bom: false,
            read_only: Some("This file isn't valid UTF-8, so it can't be edited safely.".into()),
        },
    }
}

pub fn encode(body: &str, bom: bool) -> Vec<u8> {
    let mut bytes = Vec::with_capacity(body.len() + 3);
    if bom {
        bytes.extend_from_slice(BOM.as_bytes());
    }
    bytes.extend_from_slice(body.as_bytes());
    bytes
}

/// Replaces a file in one step, so a crash can't leave it half-written.
pub fn write_atomic(path: &Path, bytes: &[u8]) -> Result<(), String> {
    let describe = |e: std::io::Error| format!("Couldn't write {}: {e}", path.display());
    let metadata = fs::metadata(path).map_err(describe)?;
    if metadata.permissions().readonly() {
        return Err(format!("{} is read-only.", path.display()));
    }
    let name = path.file_name().ok_or("path has no file name")?;
    let temp = path.with_file_name(format!(".{}{TEMP_SUFFIX}", name.to_string_lossy()));
    fs::write(&temp, bytes).map_err(describe)?;
    fs::rename(&temp, path).map_err(|e| {
        let _ = fs::remove_file(&temp);
        describe(e)
    })
}

#[derive(Clone, Debug)]
pub struct Loaded {
    pub doc: Arc<Document>,
    pub hash: u64,
    pub read_only: Option<String>,
}

/// Reads and tokenizes a file. Returns `None` if its bytes still hash to `known`.
pub fn load(
    highlighter: &Highlighter,
    path: &Path,
    theme: &str,
    known: Option<u64>,
) -> Result<Option<Loaded>, String> {
    let bytes = fs::read(path).map_err(|e| format!("Couldn't read {}: {e}", path.display()))?;
    let hash = hash_of(&bytes);
    if known == Some(hash) {
        return Ok(None);
    }
    let decoded = decode(&bytes);
    let language = highlighter.language_for(path);
    let doc = highlighter.highlight(&decoded.body, &language, theme)?;
    Ok(Some(Loaded {
        doc: Arc::new(doc),
        hash,
        read_only: decoded.read_only,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch_folder(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("commentcleanup-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn put(path: PathBuf, text: &str) {
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, text).unwrap();
    }

    fn ignored_names(root: &Path, expanded: &[&str]) -> Vec<String> {
        let expanded = expanded.iter().map(|d| root.join(d)).collect();
        let mut names: Vec<String> = list_tree(root, &expanded)
            .into_iter()
            .filter(|e| e.ignored)
            .map(|e| e.path.strip_prefix(root).unwrap().to_string_lossy().replace('\\', "/"))
            .collect();
        names.sort();
        names
    }

    #[test]
    fn gitignore_marks_files_folders_and_their_contents() {
        let root = scratch_folder("ignore");
        put(root.join(".gitignore"), "node_modules/\n*.log\n");
        put(root.join("node_modules/pkg/index.js"), "");
        put(root.join("a.log"), "");
        put(root.join("a.txt"), "");
        assert_eq!(
            ignored_names(&root, &["node_modules", "node_modules/pkg"]),
            ["a.log", "node_modules", "node_modules/pkg", "node_modules/pkg/index.js"]
        );
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn nested_gitignore_applies_only_below_itself_and_can_unignore() {
        let root = scratch_folder("nested");
        put(root.join(".gitignore"), "*.log\n");
        put(root.join("apps/web/.gitignore"), ".dev.vars\n!keep.log\n");
        put(root.join("apps/web/.dev.vars"), "");
        put(root.join("apps/web/keep.log"), "");
        put(root.join("apps/web/drop.log"), "");
        put(root.join(".dev.vars"), "");
        assert_eq!(
            ignored_names(&root, &["apps", "apps/web"]),
            ["apps/web/.dev.vars", "apps/web/drop.log"]
        );
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn opening_a_subfolder_still_honours_the_repository_rules() {
        let root = scratch_folder("repo");
        fs::create_dir_all(root.join(".git")).unwrap();
        put(root.join(".gitignore"), "*.log\n");
        put(root.join("sub/a.log"), "");
        put(root.join("sub/a.txt"), "");
        assert_eq!(ignored_names(&root.join("sub"), &[]), ["a.log"]);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn a_single_path_gets_the_same_answer_as_the_tree() {
        let root = scratch_folder("single");
        put(root.join(".gitignore"), "dist/\n");
        put(root.join("apps/.gitignore"), "*.log\n");
        put(root.join("dist/a.js"), "");
        put(root.join("apps/a.log"), "");
        put(root.join("apps/a.ts"), "");
        assert!(is_path_ignored(&root, &root.join("dist/a.js")));
        assert!(is_path_ignored(&root, &root.join("apps/a.log")));
        assert!(!is_path_ignored(&root, &root.join("apps/a.ts")));
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn bom_round_trips() {
        let bytes = "\u{feff}// a\r\n".as_bytes();
        let decoded = decode(bytes);
        assert!(decoded.bom);
        assert_eq!(decoded.body, "// a\r\n");
        assert_eq!(encode(&decoded.body, decoded.bom), bytes);
    }

    #[test]
    fn non_utf8_is_read_only() {
        assert!(decode(&[0x2f, 0x2f, 0xff, 0x0a]).read_only.is_some());
        assert!(decode(&[0xff, 0xfe, 0x2f, 0x00]).read_only.is_some());
    }
}
