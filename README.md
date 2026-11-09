# CommentCleanup

A straightforward desktop app for cleaning up code comments, made in Rust.

Open a file or folder and CommentCleanup finds every comment using real syntax grammars, so a `//` inside a string is left alone. Go through the comments one at a time, or fix a whole file or folder at once. Every change can be undone.

## Download

Get `CommentCleanup-windows-x64.exe` from the [latest release](https://github.com/Wilsoon7721/CommentCleanup/releases/latest) and run it. There is nothing to install, and the app keeps itself up to date from then on.

The exe isn't code-signed, so Windows SmartScreen may warn you the first time. Each release includes a `.sha256` file if you want to verify your download.

## What it does

**Clean up comments**

- **Fix** applies what your rules recommend: delete it, squash it onto one line, or leave it.
- **Squash**, **Delete** and **Reword** do exactly that to the selected comment.
- **Ignore** tells the app to stop flagging a comment.
- **Go Through All** walks through every comment in order, moving on after each decision.
- **Fix all** works on the whole file. Use the dropdown beside it to choose Auto fix, Delete or Squash.
- **Clean Folder** previews every change across a folder before anything is written.
- **Undo** restores a file, and **Undo Folder Cleanup** restores a whole folder run.
- The **Just Changed** pane (`Ctrl+J`, off by default) shows each change before and after, with the changed words highlighted, and lets you go back to it or undo it.

**Write better comments with AI (optional)**

- **Suggest** asks a model from Groq, OpenAI, OpenRouter, Ollama or any OpenAI-compatible server to rewrite a comment more concisely, or to explain why the code does what it does.
- Suggestions can be prepared ahead of time while you go through a file, with up to eight comments sent in one request.
- Replies are cached, and each comment is sent at most once every 30 seconds, to stay within rate limits.

**Work like an editor**

- File explorer, tabs, and files that reload when they change on disk
- Syntax highlighting using themes bundled with the app or from an installed editor
- Find in file, find in all files, and a command palette
- Git awareness, including a filter for comments on lines you've changed and a one-click commit hook
- Your session, window size and position are restored on launch

**Test your rules**

Try a comment against your rules in Settings, and see how a change would affect the open folder before you save it.

## Keyboard shortcuts

| Key     | Action                                             |     | Key            | Action                  |
| ------- | -------------------------------------------------- | --- | -------------- | ----------------------- |
| `F`     | Fix                                                |     | `G`            | Go Through All          |
| `I`     | Ignore                                             |     | `N` / `P`      | Next / previous comment |
| `S`     | Squash                                             |     | `Ctrl+Z`       | Undo                    |
| `Del`   | Delete                                             |     | `Ctrl+F`       | Find                    |
| `R`     | Reword, or Retry while a suggestion is open        |     | `Ctrl+Shift+F` | Find in all files       |
| `A`     | Suggest                                            |     | `Ctrl+Shift+P` | Command palette         |
| `Enter` | Approve a suggestion                               |     | `Ctrl+B`       | Toggle the sidebar      |
| `D`     | Include the surrounding function in the suggestion |     | `Ctrl+,`       | Settings                |

`Ctrl+O` opens a file, `Ctrl+Shift+O` opens a folder, `Ctrl+W` closes the current file and `Ctrl+J` toggles the Just Changed pane.

## Rules

Settings control which comments are protected, squashed or deleted. By default, tool directives, license notices and TODO-style notes are protected, and empty comments and divider lines are deleted.

To share rules with a project, tick **Use separate settings for this folder** in Settings and commit the `.commentcleanup.json` it creates at the root of the folder. Any setting you leave out keeps its default:

```json
{
  "protect_todos": true,
  "protect_patterns": ["HACK"],
  "squash_enabled": true,
  "squash_max_width": 120,
  "squash_max_lines": 4,
  "delete_commented_code": false
}
```

## AI setup

Choose a provider in Settings, then a model:

| Provider                 | What you need                                                                           |
| ------------------------ | --------------------------------------------------------------------------------------- |
| Groq, OpenAI, OpenRouter | An API key from that service                                                            |
| Ollama                   | Ollama running on this computer at its default address. No key needed                   |
| Custom                   | The address of any OpenAI-compatible server, ending in `/v1`, and a key if it needs one |

A custom server must use `https://`. Plain `http://` is only allowed for this computer, such as LM Studio on `localhost`, so your code and key are never sent unencrypted.

Keys are encrypted with Windows' per-user data protection and stored in `%APPDATA%\CommentCleanup`, one per provider. A key is only ever sent to its own provider.

A suggestion sends the provider the comment and the function around it. You can leave the function out in Settings, or for a single comment with `D`. Nothing is sent until you ask for a suggestion or start going through comments with **Suggest ahead** on.

## Command line check

`CommentCleanup-check-windows-x64.exe` is also on the release page. It reports what your rules would change, and never edits a file, which makes it useful for hooks and CI.

```bash
commentcleanup-check [FOLDER] [OPTIONS]
```

| Option         | Meaning                                                                             |
| -------------- | ----------------------------------------------------------------------------------- |
| `--changed`    | Only comments on lines changed since the last commit                                |
| `--staged`     | Only comments on lines staged for the next commit                                   |
| `--no-squash`  | Don't count comments that would only be squashed                                    |
| `--editors`    | Also use grammars from installed editors (results can then differ between machines) |
| `-h`, `--help` | Show help                                                                           |

| Exit code | Meaning                |
| --------- | ---------------------- |
| `0`       | Nothing to change      |
| `1`       | Something would change |
| `2`       | The check couldn't run |

Rules come from the folder's `.commentcleanup.json`, then the app's settings, then the defaults.

## Commit hook

In a Git repository, choose **Edit > Install Commit Hook** or use the button in Settings. Commits then stop when your rules would delete a comment on a staged line. Squash suggestions never stop a commit, and neither does a check that can't run.

Skip it once with `git commit --no-verify`, or remove it from the same menu. An existing pre-commit hook is never replaced. The hook runs this copy of the app with `--check`, which takes the same options as the command line check, so reinstall it if you move the exe.

## Updates

The app checks GitHub Releases for new versions, verifies each download against its SHA-256 checksum, and shows what changed after updating. You can turn off **Find and download updates automatically** in Settings and use **Check for Updates** instead. The app contains no tokens, and updates come only from this repository's public releases.

## Building from source

You'll need a recent stable [Rust](https://rustup.rs) toolchain.

```bash
cargo build --release
cargo test
```

Binaries are written to `target/release`. Local builds show the version from `Cargo.toml` and don't update themselves.

## License

[MIT](LICENSE)
