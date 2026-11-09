//! Finds newer releases on GitHub and swaps the running executable for them.

use crate::ai;
use iced::futures::Stream;
use iced::futures::channel::mpsc;
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Duration;

pub const VERSION: &str = env!("CC_VERSION");
pub const COMMIT: &str = env!("CC_COMMIT");
const REPO: &str = env!("CC_REPO");

const API_URL: &str = "https://api.github.com";
const ASSET: &str = "CommentCleanup-windows-x64.exe";
const CHECK_TIMEOUT: Duration = Duration::from_secs(20);
const DOWNLOAD_TIMEOUT: Duration = Duration::from_secs(30 * 60);
/// Longest release notes kept, so a huge changelog can't bloat the session file.
const MAX_NOTES: usize = 8000;
const USER_AGENT: &str = "CommentCleanup";

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Release {
    pub version: String,
    url: String,
    checksum_url: String,
    size: u64,
    pub notes: String,
}

#[derive(Clone, Debug)]
pub enum Progress {
    Bytes(u64, u64),
    Done(Result<PathBuf, String>),
}

/// Only builds from the release workflow know where their updates live.
pub fn available() -> bool {
    !REPO.is_empty()
}

/// A local stand-in for testing the update path end to end.
fn base_url() -> String {
    std::env::var("COMMENTCLEANUP_UPDATE_URL")
        .ok()
        .filter(|url| url.starts_with("http://127.0.0.1:") || url.starts_with("http://localhost:"))
        .unwrap_or_else(|| API_URL.to_owned())
}

fn parse_version(text: &str) -> Option<(u64, u64, u64)> {
    let mut parts = text.trim().trim_start_matches('v').split('.').map(|part| part.parse().ok());
    let version = (parts.next()??, parts.next()??, parts.next()??);
    parts.next().is_none().then_some(version)
}

fn is_newer(candidate: &str, current: &str) -> bool {
    match (parse_version(candidate), parse_version(current)) {
        (Some(candidate), Some(current)) => candidate > current,
        _ => false,
    }
}

fn get(url: &str, accept: &str, timeout: Duration) -> Result<ureq::http::Response<ureq::Body>, String> {
    let agent = ai::agent(timeout);
    let response = agent.get(url).header("User-Agent", USER_AGENT).header("Accept", accept).call().map_err(|_| "Couldn't reach GitHub. Check your connection.".to_owned())?;
    match response.status().as_u16() {
        200..=299 => Ok(response),
        403 | 429 => Err("GitHub's rate limit was reached. Try again later.".to_owned()),
        404 => Err("No release has been published yet.".to_owned()),
        status => Err(format!("GitHub returned error {status}.")),
    }
}

fn read_release(reply: &Value) -> Result<Release, String> {
    let incomplete = || "That release is missing its download.".to_owned();
    let tag = reply["tag_name"].as_str().ok_or_else(incomplete)?;
    let assets = reply["assets"].as_array().ok_or_else(incomplete)?;
    let named = |name: &str| assets.iter().find(|asset| asset["name"].as_str() == Some(name));
    let asset = named(ASSET).ok_or_else(incomplete)?;
    let checksum = named(&format!("{ASSET}.sha256")).ok_or_else(incomplete)?;
    Ok(Release {
        version: tag.trim_start_matches('v').to_owned(),
        url: asset["url"].as_str().ok_or_else(incomplete)?.to_owned(),
        checksum_url: checksum["url"].as_str().ok_or_else(incomplete)?.to_owned(),
        size: asset["size"].as_u64().ok_or_else(incomplete)?,
        notes: reply["body"].as_str().unwrap_or("").chars().take(MAX_NOTES).collect(),
    })
}

/// The latest release, if it is newer than this build.
pub fn check() -> Result<Option<Release>, String> {
    let url = format!("{}/repos/{REPO}/releases/latest", base_url());
    let mut response = get(&url, "application/vnd.github+json", CHECK_TIMEOUT)?;
    let unreadable = |_| "GitHub's reply couldn't be read.".to_owned();
    let body = response.body_mut().read_to_string().map_err(unreadable)?;
    let reply: Value = serde_json::from_str(&body).map_err(|_| "GitHub's reply couldn't be read.".to_owned())?;
    let release = read_release(&reply)?;
    Ok(is_newer(&release.version, VERSION).then_some(release))
}

fn sibling(exe: &Path, suffix: &str) -> PathBuf {
    let mut name = exe.as_os_str().to_owned();
    name.push(suffix);
    PathBuf::from(name)
}

fn current_exe() -> Result<PathBuf, String> {
    std::env::current_exe().map_err(|e| format!("Couldn't find the app's own file: {e}"))
}

/// The expected hash from a `sha256sum` line.
fn parse_checksum(text: &str) -> Option<String> {
    let hash = text.split_whitespace().next()?.to_ascii_lowercase();
    (hash.len() == 64 && hash.bytes().all(|byte| byte.is_ascii_hexdigit())).then_some(hash)
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

fn fetch(release: &Release, target: &Path, report: &impl Fn(u64)) -> Result<(), String> {
    let lost = || "The download was interrupted. Try again.".to_owned();
    let octets = "application/octet-stream";
    let mut response = get(&release.checksum_url, octets, CHECK_TIMEOUT)?;
    let listed = response.body_mut().read_to_string().map_err(|_| lost())?;
    let expected = parse_checksum(&listed).ok_or("The release's checksum couldn't be read.")?;

    let mut response = get(&release.url, octets, DOWNLOAD_TIMEOUT)?;
    let mut body = response.body_mut().with_config().limit(release.size + 1).reader();
    let mut file = std::fs::File::create(target)
        .map_err(|e| format!("Couldn't save the update next to the app: {e}"))?;
    let mut hasher = Sha256::new();
    let mut buffer = vec![0; 64 * 1024];
    let mut written = 0;
    let mut starts_as_program = false;
    loop {
        let read = body.read(&mut buffer).map_err(|_| lost())?;
        if read == 0 {
            break;
        }
        if written == 0 {
            starts_as_program = buffer[..read].starts_with(b"MZ");
        }
        hasher.update(&buffer[..read]);
        file.write_all(&buffer[..read]).map_err(|e| format!("Couldn't save the update: {e}"))?;
        written += read as u64;
        report(written);
    }
    file.sync_all().map_err(|e| format!("Couldn't save the update: {e}"))?;
    if written != release.size {
        return Err("The download was incomplete. Try again.".to_owned());
    }
    if hex(&hasher.finalize()) != expected || !starts_as_program {
        return Err("The download didn't match its checksum, so it wasn't kept.".to_owned());
    }
    Ok(())
}

/// Downloads and verifies a release beside the running executable.
pub fn download(release: Release) -> impl Stream<Item = Progress> {
    let (sender, receiver) = mpsc::unbounded();
    std::thread::spawn(move || {
        let total = release.size;
        let last = std::cell::Cell::new(u64::MAX);
        let report = |got: u64| {
            let percent = got * 100 / total.max(1);
            if last.replace(percent) != percent {
                let _ = sender.unbounded_send(Progress::Bytes(got, total));
            }
        };
        let result = current_exe().and_then(|exe| {
            let target = sibling(&exe, ".new");
            match fetch(&release, &target, &report) {
                Ok(()) => Ok(target),
                Err(error) => {
                    let _ = std::fs::remove_file(&target);
                    Err(error)
                }
            }
        });
        let _ = sender.unbounded_send(Progress::Done(result));
    });
    receiver
}

/// Puts a downloaded update in the running executable's place.
pub fn install(update: &Path) -> Result<(), String> {
    let exe = current_exe()?;
    swap(&exe, update)
}

fn swap(exe: &Path, update: &Path) -> Result<(), String> {
    let old = sibling(exe, ".old");
    let _ = std::fs::remove_file(&old);
    std::fs::rename(exe, &old).map_err(|e| format!("Couldn't replace the app: {e}"))?;
    if let Err(error) = std::fs::rename(update, exe) {
        let _ = std::fs::rename(&old, exe);
        return Err(format!("Couldn't replace the app: {error}"));
    }
    Ok(())
}

/// Starts the app again with the arguments it was given.
pub fn relaunch() -> Result<(), String> {
    Command::new(current_exe()?)
        .args(std::env::args_os().skip(1))
        .spawn()
        .map(|_| ())
        .map_err(|e| format!("The update is installed, but the app couldn't restart: {e}"))
}

/// Clears what an earlier update left behind. The previous version may still be closing as this one starts, so it gets a moment first.
pub fn clean_up() {
    let Ok(exe) = current_exe() else {
        return;
    };
    let _ = std::fs::remove_file(sibling(&exe, ".new"));
    std::thread::spawn(move || {
        std::thread::sleep(Duration::from_secs(4));
        let _ = std::fs::remove_file(sibling(&exe, ".old"));
    });
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum NoteLine {
    Heading(String),
    Bullet(String),
    Text(String),
}

/// Plain lines from GitHub's Markdown release notes.
pub fn note_lines(notes: &str) -> Vec<NoteLine> {
    let tidy = |line: &str| line.replace("**", "").replace('`', "");
    notes
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty() && !line.starts_with("**Full Changelog"))
        .map(|line| {
            if let Some(heading) = line.strip_prefix('#') {
                NoteLine::Heading(tidy(heading.trim_start_matches('#').trim()))
            } else if let Some(item) = line.strip_prefix("* ").or_else(|| line.strip_prefix("- ")) {
                // GitHub adds " by @someone in https://…", which isn't about the change.
                NoteLine::Bullet(tidy(item.split(" by @").next().unwrap_or(item)))
            } else {
                NoteLine::Text(tidy(line))
            }
        })
        .collect()
}

/// Beats on a slow timer, for checking again while the app stays open.
pub fn ticks(every: Duration) -> impl Stream<Item = ()> {
    let (sender, receiver) = mpsc::unbounded();
    std::thread::spawn(move || {
        loop {
            std::thread::sleep(every);
            if sender.unbounded_send(()).is_err() {
                break;
            }
        }
    });
    receiver
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    #[test]
    fn versions_compare_as_numbers() {
        assert!(is_newer("v0.10.0", "0.9.9"));
        assert!(is_newer("1.0.0", "0.99.99"));
        assert!(!is_newer("0.2.0", "0.2.0"));
        assert!(!is_newer("0.1.9", "0.2.0"));
        assert!(!is_newer("nightly", "0.2.0"));
        assert!(!is_newer("0.3.0-beta", "0.2.0"));
    }

    #[test]
    fn a_release_needs_both_its_program_and_its_checksum() {
        let good = serde_json::json!({"tag_name": "v1.2.3", "assets": [
            {"name": ASSET, "url": "http://x/1", "size": 5},
            {"name": format!("{ASSET}.sha256"), "url": "http://x/2", "size": 90},
        ]});
        let release = read_release(&good).unwrap();
        assert_eq!((release.version.as_str(), release.url.as_str(), release.size), ("1.2.3", "http://x/1", 5));
        let missing = serde_json::json!({"tag_name": "v1.2.3", "assets": [{"name": ASSET, "url": "u", "size": 5}]});
        assert!(read_release(&missing).is_err());
    }

    #[test]
    fn release_notes_become_plain_lines() {
        let notes = "Built from commit abc.\n\n## What's Changed\n* Add **find** by @me in https://x/1\n- Fix `exit`\n\n**Full Changelog**: https://x";
        assert_eq!(
            note_lines(notes),
            [
                NoteLine::Text("Built from commit abc.".into()),
                NoteLine::Heading("What's Changed".into()),
                NoteLine::Bullet("Add find".into()),
                NoteLine::Bullet("Fix exit".into()),
            ]
        );
    }

    #[test]
    fn checksums_are_read_from_sha256sum_output() {
        let hash = "A".repeat(64);
        assert_eq!(parse_checksum(&format!("{hash} *file.exe\n")), Some("a".repeat(64)));
        assert_eq!(parse_checksum("not-a-hash file"), None);
        assert_eq!(parse_checksum(""), None);
    }

    #[test]
    fn swapping_keeps_the_old_program_until_the_new_one_is_in_place() {
        let dir = std::env::temp_dir().join(format!("commentcleanup-update-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        let exe = dir.join("app.exe");
        let new = dir.join("app.exe.new");
        fs::write(&exe, "old").unwrap();
        fs::write(&new, "new").unwrap();
        swap(&exe, &new).unwrap();
        assert_eq!(fs::read_to_string(&exe).unwrap(), "new");
        assert_eq!(fs::read_to_string(dir.join("app.exe.old")).unwrap(), "old");
        assert!(!new.exists());

        fs::write(&exe, "current").unwrap();
        assert!(swap(&exe, &dir.join("missing")).is_err());
        assert_eq!(fs::read_to_string(&exe).unwrap(), "current", "a failed swap puts it back");
        fs::remove_dir_all(dir).unwrap();
    }
}
