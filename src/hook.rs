use crate::git;
use std::path::Path;

const MARKER: &str = "# Added by CommentCleanup";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum HookState {
    Missing,
    Ours,
    Foreign,
}

fn script(exe: &Path) -> String {
    let exe = exe.to_string_lossy().replace('\\', "/").replace('\'', r"'\''");
    format!(
        "#!/bin/sh\n\
         {MARKER}. Remove it from the Edit menu, or skip it once with git commit --no-verify.\n\
         exe='{exe}'\n\
         if [ ! -f \"$exe\" ]; then\n\
         \x20   echo \"CommentCleanup isn't at $exe any more, so comments weren't checked.\" >&2\n\
         \x20   exit 0\n\
         fi\n\
         output=$(\"$exe\" --check . --staged --no-squash 2>&1)\n\
         status=$?\n\
         if [ \"$status\" -eq 1 ]; then\n\
         \x20   printf '%s\\n' \"$output\" >&2\n\
         \x20   echo \"Clean these up in CommentCleanup, or commit with --no-verify to skip the check.\" >&2\n\
         \x20   exit 1\n\
         fi\n\
         if [ \"$status\" -ne 0 ]; then\n\
         \x20   printf 'CommentCleanup could not check this commit: %s\\n' \"$output\" >&2\n\
         fi\n\
         exit 0\n"
    )
}

fn state_of(existing: Option<&str>) -> HookState {
    match existing {
        None => HookState::Missing,
        Some(text) if text.contains(MARKER) => HookState::Ours,
        Some(_) => HookState::Foreign,
    }
}

fn read_state(hook: &Path) -> HookState {
    match std::fs::read(hook) {
        Ok(bytes) => state_of(Some(&String::from_utf8_lossy(&bytes))),
        Err(_) => HookState::Missing,
    }
}

pub fn state(root: &Path) -> Option<HookState> {
    Some(read_state(&git::pre_commit_path(root)?))
}

pub fn install(root: &Path) -> Result<HookState, String> {
    let hook = git::pre_commit_path(root).ok_or("This folder isn't in a Git repository.")?;
    if read_state(&hook) == HookState::Foreign {
        return Err("This repository already has a pre-commit hook, so it was left alone.".into());
    }
    let exe = std::env::current_exe().map_err(|e| format!("Couldn't find the app: {e}"))?;
    if let Some(folder) = hook.parent() {
        std::fs::create_dir_all(folder).map_err(|e| format!("Couldn't create the hooks folder: {e}"))?;
    }
    std::fs::write(&hook, script(&exe)).map_err(|e| format!("Couldn't write the hook: {e}"))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&hook, std::fs::Permissions::from_mode(0o755))
            .map_err(|e| format!("Couldn't make the hook runnable: {e}"))?;
    }
    Ok(HookState::Ours)
}

pub fn remove(root: &Path) -> Result<HookState, String> {
    let hook = git::pre_commit_path(root).ok_or("This folder isn't in a Git repository.")?;
    if read_state(&hook) != HookState::Ours {
        return Err("That pre-commit hook wasn't added by CommentCleanup, so it was left alone.".into());
    }
    std::fs::remove_file(&hook).map_err(|e| format!("Couldn't remove the hook: {e}"))?;
    Ok(HookState::Missing)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_a_hook_with_the_marker_counts_as_ours() {
        assert_eq!(state_of(None), HookState::Missing);
        assert_eq!(state_of(Some(&script(Path::new("C:/a.exe")))), HookState::Ours);
        assert_eq!(state_of(Some("#!/bin/sh\nnpm test\n")), HookState::Foreign);
    }

    #[test]
    fn the_script_quotes_the_path_and_uses_unix_lines() {
        let text = script(Path::new(r"C:\it's here\app.exe"));
        assert!(text.contains(r"exe='C:/it'\''s here/app.exe'"));
        assert!(!text.contains('\r'));
    }

    #[test]
    fn only_a_real_finding_stops_the_commit() {
        let text = script(Path::new("C:/a.exe"));
        assert!(text.contains("\"$status\" -eq 1"));
        assert_eq!(text.matches("exit 1").count(), 1);
    }
}
