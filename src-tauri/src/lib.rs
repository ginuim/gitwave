use base64::{engine::general_purpose::STANDARD, Engine};
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::Mutex;
use tauri::{Manager, State};
use tauri_plugin_dialog::DialogExt;
use tokio::sync::oneshot;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FileStatus {
    pub path: String,
    pub status: String,
    pub is_staged: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BranchInfo {
    pub name: String,
    pub is_current: bool,
    pub is_remote: bool,
    pub is_head: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CommitLog {
    pub hash: String,
    pub author: String,
    pub date: String,
    pub message: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StashEntry {
    pub index: usize,
    pub message: String,
    pub branch: String,
}

pub struct AppState {
    repo_path: Mutex<Option<String>>,
}

fn file_path_to_string(fp: tauri_plugin_dialog::FilePath) -> String {
    fp.to_string()
}

fn normalize_path_for_git(path: &str) -> String {
    #[cfg(windows)]
    {
        path.replace('\\', "/")
    }
    #[cfg(not(windows))]
    {
        path.to_string()
    }
}

fn set_git_utf8_env(cmd: &mut Command) {
    cmd.env("LANG", "en_US.UTF-8");
    cmd.env("LC_ALL", "en_US.UTF-8");
    #[cfg(windows)]
    {
        cmd.env("GIT_UTF8_PATH", "1");
    }
}

/// Windows GUI 进程默认会给子进程分配控制台，导致每次 `git` 都会闪一下黑框。
fn hide_git_child_console(cmd: &mut Command) {
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        cmd.creation_flags(CREATE_NO_WINDOW);
    }
    #[cfg(not(windows))]
    {
        let _ = cmd;
    }
}

fn run_git(repo: &str, args: &[&str]) -> Result<String, String> {
    let mut cmd = Command::new("git");
    hide_git_child_console(&mut cmd);
    cmd.current_dir(repo);
    cmd.args(args);
    set_git_utf8_env(&mut cmd);
    let output = cmd
        .output()
        .map_err(|e| format!("failed to spawn git: {e}"))?;
    if !output.status.success() {
        return Err(String::from_utf8_lossy(&output.stderr).into_owned());
    }
    Ok(String::from_utf8_lossy(&output.stdout).into_owned())
}

fn run_git_bytes(repo: &str, args: &[&str]) -> Result<Vec<u8>, String> {
    let mut cmd = Command::new("git");
    hide_git_child_console(&mut cmd);
    cmd.current_dir(repo);
    cmd.args(args);
    set_git_utf8_env(&mut cmd);
    let output = cmd
        .output()
        .map_err(|e| format!("failed to spawn git: {e}"))?;
    if !output.status.success() {
        return Err(String::from_utf8_lossy(&output.stderr).into_owned());
    }
    Ok(output.stdout)
}

fn ensure_safe_repo_relative_path(path: &str) -> Result<(), String> {
    let p = normalize_path_for_git(path);
    if p.is_empty() || p.starts_with('/') || p.contains("..") {
        return Err("invalid path".to_string());
    }
    Ok(())
}

fn mime_for_image_path(path: &str) -> &'static str {
    let lower = path.to_ascii_lowercase();
    if lower.ends_with(".png") {
        "image/png"
    } else if lower.ends_with(".jpg") || lower.ends_with(".jpeg") {
        "image/jpeg"
    } else if lower.ends_with(".gif") {
        "image/gif"
    } else if lower.ends_with(".webp") {
        "image/webp"
    } else if lower.ends_with(".bmp") {
        "image/bmp"
    } else if lower.ends_with(".ico") {
        "image/x-icon"
    } else {
        "application/octet-stream"
    }
}

fn bytes_to_data_url(bytes: &[u8], mime: &str) -> String {
    format!("data:{mime};base64,{}", STANDARD.encode(bytes))
}

/// Resolve `rev:path` (e.g. `HEAD:src/a.png`, `:0:src/a.png`, `abc123^:src/a.png`) to blob bytes.
fn read_git_blob_bytes(repo: &str, rev_path: &str) -> Result<Option<Vec<u8>>, String> {
    let mut cmd = Command::new("git");
    hide_git_child_console(&mut cmd);
    cmd.current_dir(repo);
    cmd.args(["rev-parse", "-q", "--verify", rev_path]);
    set_git_utf8_env(&mut cmd);
    let out = cmd
        .output()
        .map_err(|e| format!("failed to spawn git: {e}"))?;
    if !out.status.success() {
        return Ok(None);
    }
    let hash = String::from_utf8_lossy(&out.stdout).trim().to_string();
    if hash.is_empty() {
        return Ok(None);
    }
    let bytes = run_git_bytes(repo, &["cat-file", "blob", &hash])?;
    Ok(Some(bytes))
}

fn read_worktree_file_bytes(repo: &str, rel: &str) -> Result<Option<Vec<u8>>, String> {
    ensure_safe_repo_relative_path(rel)?;
    let root = Path::new(repo);
    let path = root.join(rel);
    if !path.exists() || !path.is_file() {
        return Ok(None);
    }
    let root_canon = root
        .canonicalize()
        .map_err(|e| format!("failed to canonicalize repo root: {e}"))?;
    let file_canon = path
        .canonicalize()
        .map_err(|e| format!("failed to canonicalize file path: {e}"))?;
    if !file_canon.starts_with(&root_canon) {
        return Err("path escapes repository".to_string());
    }
    Ok(Some(fs::read(&file_canon).map_err(|e| format!("read file: {e}"))?))
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BinaryImagePreview {
    pub old_data_url: Option<String>,
    pub new_data_url: Option<String>,
}

/// `kind`: `unstaged` (worktree vs index), `staged` (index vs HEAD), `commit` (parent vs commit).
#[tauri::command]
fn get_binary_image_preview(
    state: State<'_, AppState>,
    relative_path: String,
    kind: String,
    commit_hash: Option<String>,
) -> Result<BinaryImagePreview, String> {
    let repo = require_repo(&state)?;
    let rel = normalize_path_for_git(&relative_path);
    ensure_safe_repo_relative_path(&rel)?;
    let mime = mime_for_image_path(&rel);

    match kind.as_str() {
        "unstaged" => {
            let old = read_git_blob_bytes(&repo, &format!(":0:{rel}"))?
                .map(|b| bytes_to_data_url(&b, mime));
            let new = read_worktree_file_bytes(&repo, &rel)?
                .map(|b| bytes_to_data_url(&b, mime));
            Ok(BinaryImagePreview {
                old_data_url: old,
                new_data_url: new,
            })
        }
        "staged" => {
            let old = read_git_blob_bytes(&repo, &format!("HEAD:{rel}"))?
                .map(|b| bytes_to_data_url(&b, mime));
            let new = read_git_blob_bytes(&repo, &format!(":0:{rel}"))?
                .map(|b| bytes_to_data_url(&b, mime));
            Ok(BinaryImagePreview {
                old_data_url: old,
                new_data_url: new,
            })
        }
        "commit" => {
            let hash = commit_hash.ok_or_else(|| "commitHash required for commit preview".to_string())?;
            let old = read_git_blob_bytes(&repo, &format!("{hash}^:{rel}"))?
                .map(|b| bytes_to_data_url(&b, mime));
            let new = read_git_blob_bytes(&repo, &format!("{hash}:{rel}"))?
                .map(|b| bytes_to_data_url(&b, mime));
            Ok(BinaryImagePreview {
                old_data_url: old,
                new_data_url: new,
            })
        }
        _ => Err(format!("unknown preview kind: {kind}")),
    }
}

fn require_repo(state: &AppState) -> Result<String, String> {
    let guard = state.repo_path.lock().map_err(|_| "state lock poisoned")?;
    guard
        .clone()
        .ok_or_else(|| "no repository open".to_string())
}

fn is_git_dir(root: &Path) -> bool {
    let git = root.join(".git");
    git.exists()
}

fn repos_file_path(app: &tauri::AppHandle) -> Result<PathBuf, String> {
    let dir = app
        .path()
        .app_data_dir()
        .map_err(|e| format!("failed to get app data dir: {e}"))?;
    fs::create_dir_all(&dir).map_err(|e| format!("failed to create app data dir: {e}"))?;
    Ok(dir.join("recent_repos.json"))
}

fn load_recent_repos(app: &tauri::AppHandle) -> Vec<String> {
    repos_file_path(app)
        .ok()
        .and_then(|path| {
            if path.exists() {
                fs::read_to_string(&path)
                    .ok()
                    .and_then(|s| serde_json::from_str(&s).ok())
            } else {
                Some(Vec::new())
            }
        })
        .unwrap_or_default()
}

fn save_recent_repos(app: &tauri::AppHandle, repos: &[String]) -> Result<(), String> {
    let path = repos_file_path(app)?;
    let json =
        serde_json::to_string(repos).map_err(|e| format!("serialization error: {e}"))?;
    fs::write(&path, &json).map_err(|e| format!("write error: {e}"))?;
    Ok(())
}

fn last_repo_file_path(app: &tauri::AppHandle) -> Result<PathBuf, String> {
    let dir = app
        .path()
        .app_data_dir()
        .map_err(|e| format!("failed to get app data dir: {e}"))?;
    fs::create_dir_all(&dir).map_err(|e| format!("failed to create app data dir: {e}"))?;
    Ok(dir.join("last_repo.txt"))
}

fn save_last_repo(app: &tauri::AppHandle, path: &str) -> Result<(), String> {
    let path_buf = last_repo_file_path(app)?;
    fs::write(&path_buf, path).map_err(|e| format!("write error: {e}"))?;
    Ok(())
}

fn load_last_repo(app: &tauri::AppHandle) -> Option<String> {
    last_repo_file_path(app).ok().and_then(|p| {
        if p.exists() {
            fs::read_to_string(&p).ok().map(|s| s.trim().to_string())
        } else {
            None
        }
    })
}

fn pinned_file_path(app: &tauri::AppHandle) -> Result<PathBuf, String> {
    let dir = app
        .path()
        .app_data_dir()
        .map_err(|e| format!("failed to get app data dir: {e}"))?;
    fs::create_dir_all(&dir).map_err(|e| format!("failed to create app data dir: {e}"))?;
    Ok(dir.join("pinned_branches.json"))
}

fn load_pinned_branches(app: &tauri::AppHandle) -> Vec<String> {
    pinned_file_path(app)
        .ok()
        .and_then(|path| {
            if path.exists() {
                fs::read_to_string(&path)
                    .ok()
                    .and_then(|s| serde_json::from_str(&s).ok())
            } else {
                Some(Vec::new())
            }
        })
        .unwrap_or_default()
}

fn save_pinned_branches(app: &tauri::AppHandle, branches: &[String]) -> Result<(), String> {
    let path = pinned_file_path(app)?;
    let json =
        serde_json::to_string(branches).map_err(|e| format!("serialization error: {e}"))?;
    fs::write(&path, &json).map_err(|e| format!("write error: {e}"))?;
    Ok(())
}


fn parse_porcelain_path(rest: &str) -> String {
    let rest = rest.trim_start();
    if let Some(pos) = rest.rfind(" -> ") {
        rest[pos + 4..].trim().to_string()
    } else {
        rest.to_string()
    }
}

fn push_status_entries(
    out: &mut Vec<FileStatus>,
    path: String,
    index: char,
    worktree: char,
) {
    let status_label = format!("{index}{worktree}");
    let staged = index != ' ' && index != '?';
    let unstaged = worktree != ' ';
    if staged {
        out.push(FileStatus {
            path: path.clone(),
            status: status_label.clone(),
            is_staged: true,
        });
    }
    if unstaged {
        out.push(FileStatus {
            path,
            status: status_label,
            is_staged: false,
        });
    }
}

fn parse_branches(raw: &str) -> Vec<BranchInfo> {
    // First pass: collect HEAD targets from symbolic refs
    let mut head_targets: Vec<String> = Vec::new();
    for line in raw.lines() {
        let trimmed = line.trim();
        if trimmed.contains(" -> ") {
            // e.g. "remotes/origin/HEAD -> origin/main"
            let pos = trimmed.find(" -> ").unwrap();
            let target = trimmed[pos + 4..].trim();
            head_targets.push(target.to_string());
        }
    }

    // Second pass: build branch list, skip symbolic ref lines
    let mut out = Vec::new();
    for line in raw.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() || trimmed.starts_with("warning:") {
            continue;
        }
        // Skip symbolic refs (HEAD -> lines)
        if trimmed.contains(" -> ") {
            continue;
        }
        let is_current = line.starts_with('*');
        let name = if is_current { &line[2..] } else { trimmed };
        let is_remote = name.starts_with("remotes/");
        let display_name = if is_remote { &name[8..] } else { name };
        let is_head = if is_remote {
            false
        } else {
            head_targets.iter().any(|t| {
                let target_branch = t.split_once('/').map(|(_, b)| b).unwrap_or("");
                target_branch == name
            })
        };
        out.push(BranchInfo {
            name: display_name.to_string(),
            is_current,
            is_remote,
            is_head,
        });
    }
    out
}

fn parse_git_status_porcelain(raw: &str) -> Vec<FileStatus> {
    let mut out = Vec::new();
    for line in raw.lines() {
        let line = line.trim_end();
        if line.is_empty() {
            continue;
        }
        let bytes = line.as_bytes();
        if bytes.len() < 3 {
            continue;
        }
        let index = bytes[0] as char;
        let worktree = bytes[1] as char;
        let sep = bytes[2] as char;
        if sep != ' ' && sep != '\t' {
            continue;
        }
        let rest = line.get(3..).unwrap_or("");
        let path = parse_porcelain_path(rest);
        if path.is_empty() {
            continue;
        }
        push_status_entries(&mut out, path, index, worktree);
    }
    out
}

#[tauri::command]
async fn open_repository(app: tauri::AppHandle) -> Result<String, String> {
    let (tx, rx) = oneshot::channel();

    app.dialog()
        .file()
        .pick_folder(move |file_path| {
            let _ = tx.send(file_path);
        });

    let picked = rx
        .await
        .map_err(|_| "dialog cancelled".to_string())?
        .ok_or_else(|| "dialog cancelled".to_string())?;

    let path_str = file_path_to_string(picked);
    let root = PathBuf::from(&path_str);
    if !is_git_dir(&root) {
        return Err(format!(
            "not a git repository (no .git at): {}",
            path_str
        ));
    }
    {
        let mut recent = load_recent_repos(&app);
        recent.retain(|r| r != &path_str);
        recent.insert(0, path_str.clone());
        recent.truncate(10);
        save_recent_repos(&app, &recent)?;

        let state = app.state::<AppState>();
        let mut guard = state.repo_path.lock().map_err(|_| "state lock poisoned")?;
        *guard = Some(path_str.clone());
    }
    save_last_repo(&app, &path_str)?;
    Ok(path_str)
}

#[tauri::command]
fn get_repo_path(state: State<'_, AppState>, app: tauri::AppHandle) -> Result<Option<String>, String> {
    let mut guard = state.repo_path.lock().map_err(|_| "state lock poisoned")?;
    if guard.is_none() {
        *guard = load_last_repo(&app);
    }
    Ok(guard.clone())
}

#[tauri::command]
fn get_recent_repos(app: tauri::AppHandle) -> Result<Vec<String>, String> {
    Ok(load_recent_repos(&app))
}

#[tauri::command]
fn switch_repository(app: tauri::AppHandle, path: String) -> Result<String, String> {
    let root = PathBuf::from(&path);
    if !is_git_dir(&root) {
        return Err(format!("not a git repository (no .git at): {path}"));
    }
    {
        let state = app.state::<AppState>();
        let mut guard = state.repo_path.lock().map_err(|_| "state lock poisoned")?;
        *guard = Some(path.clone());
    }
    save_last_repo(&app, &path)?;
    Ok(path)
}

#[tauri::command]
fn get_git_status(state: State<'_, AppState>) -> Result<Vec<FileStatus>, String> {
    let repo = require_repo(&state)?;
    let raw = run_git(&repo, &["status", "--porcelain"])?;
    Ok(parse_git_status_porcelain(&raw))
}

#[tauri::command]
fn stage_file(state: State<'_, AppState>, path: String) -> Result<(), String> {
    let repo = require_repo(&state)?;
    let p = normalize_path_for_git(&path);
    run_git(&repo, &["add", "--", &p])?;
    Ok(())
}

#[tauri::command]
fn unstage_file(state: State<'_, AppState>, path: String) -> Result<(), String> {
    let repo = require_repo(&state)?;
    let p = normalize_path_for_git(&path);
    run_git(&repo, &["reset", "HEAD", "--", &p])?;
    Ok(())
}

#[tauri::command]
fn commit_changes(state: State<'_, AppState>, message: String) -> Result<(), String> {
    let repo = require_repo(&state)?;
    if message.trim().is_empty() {
        return Err("empty commit message".to_string());
    }
    run_git(&repo, &["commit", "-m", &message])?;
    Ok(())
}

/// 是否在索引中（未跟踪文件 `git diff -- path` 恒为空，需走 `--no-index`）。
fn is_tracked_in_index(repo: &str, rel: &str) -> Result<bool, String> {
    let out = run_git(repo, &["ls-files", "--", rel])?;
    Ok(!out.trim().is_empty())
}

#[cfg(unix)]
const GIT_NULL_PATH: &str = "/dev/null";
#[cfg(windows)]
const GIT_NULL_PATH: &str = "NUL";

#[tauri::command]
fn get_file_diff(state: State<'_, AppState>, path: String, is_staged: bool) -> Result<String, String> {
    let repo = require_repo(&state)?;
    let p = normalize_path_for_git(&path);
    ensure_safe_repo_relative_path(&p)?;
    let raw = if is_staged {
        run_git(&repo, &["diff", "--cached", "--", &p])?
    } else {
        let diff = run_git(&repo, &["diff", "--", &p])?;
        if !diff.is_empty() {
            diff
        } else if !is_tracked_in_index(&repo, &p)? {
            let worktree = Path::new(&repo).join(&p);
            if worktree.is_file() {
                run_git(
                    &repo,
                    &["diff", "--no-index", "--", GIT_NULL_PATH, &p],
                )?
            } else {
                diff
            }
        } else {
            diff
        }
    };
    Ok(raw)
}

#[tauri::command]
fn get_git_log(state: State<'_, AppState>, all: Option<bool>) -> Result<Vec<CommitLog>, String> {
    let repo = require_repo(&state)?;
    let format_str = "%h|%an|%ad|%s";
    let pretty = format!("--pretty=format:{format_str}");
    let mut args = vec!["log", "-n", "50", &pretty, "--date=iso"];
    if all.unwrap_or(false) {
        args.push("--all");
    }
    let raw = run_git(&repo, &args)?;
    let mut logs = Vec::new();
    for line in raw.lines() {
        if line.is_empty() {
            continue;
        }
        let mut parts = line.splitn(4, '|');
        let hash = parts.next().unwrap_or("").to_string();
        let author = parts.next().unwrap_or("").to_string();
        let date = parts.next().unwrap_or("").to_string();
        let message = parts.next().unwrap_or("").to_string();
        logs.push(CommitLog {
            hash,
            author,
            date,
            message,
        });
    }
    Ok(logs)
}

#[tauri::command]
fn get_branches(state: State<'_, AppState>) -> Result<Vec<BranchInfo>, String> {
    let repo = require_repo(&state)?;
    let raw = run_git(&repo, &["branch", "-a"])?;
    Ok(parse_branches(&raw))
}

#[tauri::command]
fn get_commit_diff(state: State<'_, AppState>, hash: String) -> Result<String, String> {
    let repo = require_repo(&state)?;
    run_git(&repo, &["show", &hash])
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AheadBehind {
    pub ahead: usize,
    pub behind: usize,
}

#[tauri::command]
fn rename_branch(state: State<'_, AppState>, old_name: String, new_name: String) -> Result<String, String> {
    let repo = require_repo(&state)?;
    run_git(&repo, &["branch", "-m", &old_name, &new_name])
}

#[tauri::command]
fn delete_branch(state: State<'_, AppState>, name: String, force: bool) -> Result<String, String> {
    let repo = require_repo(&state)?;
    if force {
        run_git(&repo, &["branch", "-D", &name])
    } else {
        run_git(&repo, &["branch", "-d", &name])
    }
}

#[tauri::command]
fn merge_branch(state: State<'_, AppState>, name: String) -> Result<String, String> {
    let repo = require_repo(&state)?;
    run_git(&repo, &["merge", &name])
}

#[tauri::command]
fn create_branch(state: State<'_, AppState>, name: String) -> Result<String, String> {
    let repo = require_repo(&state)?;
    run_git(&repo, &["checkout", "-b", &name])
}

#[tauri::command]
fn checkout_branch(state: State<'_, AppState>, name: String) -> Result<String, String> {
    let repo = require_repo(&state)?;
    run_git(&repo, &["checkout", &name])
}

#[tauri::command]
fn checkout_remote_branch(state: State<'_, AppState>, remote: String) -> Result<String, String> {
    let repo = require_repo(&state)?;
    let local = remote.split('/').last().unwrap_or(&remote);
    run_git(&repo, &["checkout", "-b", local, "--track", &remote])
}

#[tauri::command]
async fn git_fetch(state: State<'_, AppState>) -> Result<String, String> {
    let repo = require_repo(&state)?;
    let result = tokio::task::spawn_blocking(move || run_git(&repo, &["fetch"]))
        .await
        .map_err(|e| format!("task failed: {e}"))?;
    result.map(|s| if s.is_empty() { "ok".into() } else { s })
}

#[tauri::command]
fn stage_patch(state: State<'_, AppState>, patch: String) -> Result<String, String> {
    let repo = require_repo(&state)?;
    let tmp = std::env::temp_dir().join(format!("gitwave_patch_{}", std::process::id()));
    std::fs::write(&tmp, &patch).map_err(|e| format!("failed to write patch: {e}"))?;
    let result = run_git(&repo, &["apply", "--cached", tmp.to_str().unwrap()]);
    let _ = std::fs::remove_file(&tmp);
    result
}

#[tauri::command]
fn get_ahead_behind(state: State<'_, AppState>) -> Result<AheadBehind, String> {
    let repo = require_repo(&state)?;
    let branch = run_git(&repo, &["rev-parse", "--abbrev-ref", "HEAD"])?;
    let branch = branch.trim().to_string();
    if branch == "HEAD" {
        return Ok(AheadBehind { ahead: 0, behind: 0 });
    }
    let upstream = match run_git(&repo, &["rev-parse", "--abbrev-ref", "--symbolic-full-name", "@{upstream}"]) {
        Ok(u) => u.trim().to_string(),
        Err(_) => return Ok(AheadBehind { ahead: 0, behind: 0 }),
    };
    let output = run_git(&repo, &["rev-list", "--count", "--left-right", &format!("{upstream}...HEAD")])?;
    let trimmed = output.trim();
    let parts: Vec<&str> = trimmed.split('\t').collect();
    let behind = parts.first().unwrap_or(&"0").parse().unwrap_or(0);
    let ahead = parts.get(1).copied().unwrap_or("0").parse().unwrap_or(0);
    Ok(AheadBehind { ahead, behind })
}

#[tauri::command]
async fn git_push(state: State<'_, AppState>) -> Result<String, String> {
    let repo = require_repo(&state)?;
    let result = tokio::task::spawn_blocking(move || run_git(&repo, &["push"]))
        .await
        .map_err(|e| format!("task failed: {e}"))?;
    result.map(|s| if s.is_empty() { "ok".into() } else { s })
}

#[tauri::command]
async fn git_pull(state: State<'_, AppState>) -> Result<String, String> {
    let repo = require_repo(&state)?;
    let result = tokio::task::spawn_blocking(move || run_git(&repo, &["pull"]))
        .await
        .map_err(|e| format!("task failed: {e}"))?;
    result.map(|s| if s.is_empty() { "ok".into() } else { s })
}

// === Pin branches ===

#[tauri::command]
fn pin_branch(app: tauri::AppHandle, branch: String) -> Result<(), String> {
    let mut branches = load_pinned_branches(&app);
    if !branches.contains(&branch) {
        branches.push(branch);
        save_pinned_branches(&app, &branches)?;
    }
    Ok(())
}

#[tauri::command]
fn unpin_branch(app: tauri::AppHandle, branch: String) -> Result<(), String> {
    let mut branches = load_pinned_branches(&app);
    branches.retain(|b| b != &branch);
    save_pinned_branches(&app, &branches)?;
    Ok(())
}

#[tauri::command]
fn get_pinned_branches(app: tauri::AppHandle) -> Result<Vec<String>, String> {
    Ok(load_pinned_branches(&app))
}

// === Tags ===

#[tauri::command]
fn create_tag(state: State<'_, AppState>, name: String, message: Option<String>) -> Result<String, String> {
    let repo = require_repo(&state)?;
    let mut args = vec!["tag"];
    if let Some(msg) = &message {
        if !msg.trim().is_empty() {
            args.push("-a");
            args.push(&name);
            args.push("-m");
            args.push(msg);
        } else {
            args.push(&name);
        }
    } else {
        args.push(&name);
    }
    run_git(&repo, &args)
}

// === Stash ===

#[tauri::command]
fn stash_save(state: State<'_, AppState>, message: Option<String>, include_untracked: bool) -> Result<String, String> {
    let repo = require_repo(&state)?;
    let mut args = vec!["stash", "push"];
    if include_untracked {
        args.push("--include-untracked");
    }
    if let Some(msg) = &message {
        if !msg.trim().is_empty() {
            args.push("-m");
            args.push(msg);
        }
    }
    run_git(&repo, &args)
}

#[tauri::command]
fn stash_list(state: State<'_, AppState>) -> Result<Vec<StashEntry>, String> {
    let repo = require_repo(&state)?;
    let raw = run_git(&repo, &["stash", "list"])?;
    let mut entries = Vec::new();
    for line in raw.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() { continue; }
        // stash@{0}: On branch-name: message
        let index = entries.len();
        let rest = trimmed.split_once(": ").map(|(_, r)| r).unwrap_or(trimmed);
        let branch = rest.split_once(": ").map(|(_, _r)| {
            // Try to extract branch name from "On branch-name: message"
            let branch_part = rest.strip_prefix("On ").and_then(|s| s.split_once(": ")).map(|(b, _)| b.to_string());
            let msg = rest.split_once(": ").map(|(_, m)| m.to_string()).unwrap_or_default();
            (branch_part.unwrap_or_default(), msg)
        }).unwrap_or((String::new(), rest.to_string()));
        entries.push(StashEntry {
            index,
            message: branch.1,
            branch: branch.0,
        });
    }
    Ok(entries)
}

#[tauri::command]
fn stash_apply(state: State<'_, AppState>, index: usize) -> Result<String, String> {
    let repo = require_repo(&state)?;
    run_git(&repo, &["stash", "apply", &format!("stash@{{{index}}}")])
}

#[tauri::command]
fn stash_file(state: State<'_, AppState>, path: String) -> Result<String, String> {
    let repo = require_repo(&state)?;
    run_git(&repo, &["stash", "push", "--", &path])
}

#[tauri::command]
fn stash_drop(state: State<'_, AppState>, index: usize) -> Result<String, String> {
    let repo = require_repo(&state)?;
    run_git(&repo, &["stash", "drop", &format!("stash@{{{index}}}")])
}

// === AI Commit ===

#[tauri::command]
fn get_staged_diff(state: State<'_, AppState>) -> Result<String, String> {
    let repo = require_repo(&state)?;
    run_git(&repo, &["diff", "--cached"])
}

// === Settings ===

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GitConfig {
    pub user_name: String,
    pub user_email: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProviderConfig {
    pub id: String,
    pub name: String,
    #[serde(rename = "type")]
    pub provider_type: String,
    pub base_url: String,
    pub api_key: String,
    pub is_default: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ModelConfig {
    pub id: String,
    pub provider_id: String,
    pub name: String,
    pub is_default: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GeneralSettings {
    pub user_name: String,
    pub user_email: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PromptSettings {
    pub commit_prompt: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AppSettings {
    pub general: GeneralSettings,
    pub providers: Vec<ProviderConfig>,
    pub models: Vec<ModelConfig>,
    pub prompts: PromptSettings,
}

impl Default for AppSettings {
    fn default() -> Self {
        Self {
            general: GeneralSettings {
                user_name: String::new(),
                user_email: String::new(),
            },
            providers: vec![
                ProviderConfig {
                    id: "openai-default".to_string(),
                    name: "OpenAI".to_string(),
                    provider_type: "openai".to_string(),
                    base_url: "https://api.openai.com/v1".to_string(),
                    api_key: String::new(),
                    is_default: true,
                },
                ProviderConfig {
                    id: "anthropic-default".to_string(),
                    name: "Anthropic".to_string(),
                    provider_type: "anthropic".to_string(),
                    base_url: "https://api.anthropic.com".to_string(),
                    api_key: String::new(),
                    is_default: false,
                },
            ],
            models: vec![
                ModelConfig {
                    id: "model-openai-1".to_string(),
                    provider_id: "openai-default".to_string(),
                    name: "gpt-4o".to_string(),
                    is_default: true,
                },
                ModelConfig {
                    id: "model-anthropic-1".to_string(),
                    provider_id: "anthropic-default".to_string(),
                    name: "claude-sonnet-4-20250514".to_string(),
                    is_default: false,
                },
            ],
            prompts: PromptSettings {
                commit_prompt: r#"## Format Template
你生成的 Commit Message 必须严格遵循以下结构：
<type>(<scope>): <subject>

<BLANK LINE>

<body>

<BLANK LINE>

<footer>

- `<type>`: 提交的类型（必填）
- `<scope>`: 影响的范围（可选）
- `<subject>`: 简短的修改描述（必填）
- `<body>`: 详细的修改背景和逻辑（可选）
- `<footer>`: 关联的 Issue 或破坏性变更提示（可选）

## Allowed Types (类型定义)
你只能从以下类型中选择最符合的一个填入 `<type>`：
- `feat`: 新增功能 (Feature)
- `fix`: 修复 Bug (Bugfix)
- `docs`: 仅修改文档 (Documentation)
- `style`: 代码格式调整（不影响代码运行，如空格、缩进、分号等）
- `refactor`: 代码重构（既不是新增功能，也不是修复 Bug 的代码更改）
- `perf`: 性能优化 (Performance)
- `test`: 新增或修改测试用例
- `build`: 影响构建系统或外部依赖的更改（如 npm, webpack, maven 等）
- `ci`: 更改 CI 配置或脚本（如 GitHub Actions, Travis 等）
- `chore`: 杂项（如日常事务、构建过程或辅助工具的变动，不修改 src 或 test 文件）
- `revert`: 回滚之前的提交

## Rules & Constraints (严格遵守的规则)
1. **语言设定**: Commit Message 默认使用 [英文] 编写（除非用户明确要求使用中文）。
2. **Subject 规则**:
   - 长度不得超过 50 个字符。
   - 使用祈使句（如 "add", "fix", "change"，不要使用 "added", "fixes"）。
   - 首字母小写。
   - 结尾不要加句号（`.`）。
3. **Scope 规则**: 提取修改最集中的模块名，如 `auth`, `db`, `ui`。如果是全局修改或难以归类，请省略 `(scope)`。
4. **Body 规则**:
   - 如果修改较复杂，必须提供 Body。
   - 重点解释 **"为什么做这个修改 (Why)"** 以及 **"主要逻辑是什么 (How)"**，而不是简单重复代码变动 (What)。
   - 每行不超过 72 个字符，方便终端阅读。
5. **破坏性变更**: 如果包含破坏性变更（Breaking Changes），必须在 Footer 区域以 `BREAKING CHANGE:` 开头并详细说明。
6. **Commit Message 语言**: 必须使用英文编写 Commit Message。
7. **提交范围**: 只应该提交已经git add 的代码

## Workflow (你的思考过程)
在生成结果前，请按照以下步骤在后台静默思考：
1. 分析 Diff/描述：这段代码实际改变了什么？
2. 判定类型：这是新功能、修复，还是重构？（选择最核心的 Type）
3. 提取范围：主要影响了哪个特定模块？（可选）
4. 撰写摘要：用最精炼的动宾短语描述变动。
5. 补充细节：如果是复杂变动，提炼出 1-3 点修改原因放入 Body。

## Examples (参考示例)
✅ 好的示例 1（简单的新功能）:
feat(auth): add JWT token validation for API routes

✅ 好的示例 2（包含 Body 和 Footer 的 Bug 修复）:
fix(cart): resolve incorrect total price calculation

The discount multiplier was being applied before tax, causing a 2% discrepancy in the final cart total. Moved the discount logic to execute after tax calculation.

Closes #123

✅ 好的示例 3（破坏性变更）:
refactor(api): rename user endpoint and update payload structure

BREAKING CHANGE: The endpoint `/api/v1/user` has been renamed to `/api/v1/users`. The `id` field in the payload is now required.

❌ 坏的示例（绝对不要这样做）:
- `fixed bug` (缺少 type，没有说明修复了什么)
- `feat: added new login page.` (使用了过去式 added，结尾有句号)
- `update config` (缺少具体的 type，不够清晰)

## 示例
```bash
./git-commit.sh "fix: 修复登录 bug"
./git-commit.sh "feat: 新增用户管理功能"
./git-commit.sh "docs: 更新文档"
```"#.to_string(),
            },
        }
    }
}

fn settings_path(app: &tauri::AppHandle) -> Result<PathBuf, String> {
    let data_dir = app
        .path()
        .app_data_dir()
        .map_err(|e| format!("failed to get app data dir: {e}"))?;
    fs::create_dir_all(&data_dir).map_err(|e| format!("failed to create app data dir: {e}"))?;
    Ok(data_dir.join("settings.json"))
}

#[tauri::command]
fn get_git_config(state: State<'_, AppState>) -> Result<GitConfig, String> {
    let repo = require_repo(&state)?;
    let name = run_git(&repo, &["config", "user.name"]).unwrap_or_default();
    let email = run_git(&repo, &["config", "user.email"]).unwrap_or_default();
    Ok(GitConfig {
        user_name: name.trim().to_string(),
        user_email: email.trim().to_string(),
    })
}

#[tauri::command]
fn set_git_config(state: State<'_, AppState>, user_name: String, user_email: String) -> Result<(), String> {
    let repo = require_repo(&state)?;
    if !user_name.is_empty() {
        run_git(&repo, &["config", "user.name", &user_name])?;
    }
    if !user_email.is_empty() {
        run_git(&repo, &["config", "user.email", &user_email])?;
    }
    Ok(())
}

#[tauri::command]
fn load_settings(app: tauri::AppHandle) -> Result<AppSettings, String> {
    let path = settings_path(&app)?;
    if !path.exists() {
        return Ok(AppSettings::default());
    }
    let content = fs::read_to_string(&path)
        .map_err(|e| format!("failed to read settings: {e}"))?;
    serde_json::from_str(&content).map_err(|e| format!("failed to parse settings: {e}"))
}

#[tauri::command]
fn save_settings(app: tauri::AppHandle, settings: AppSettings) -> Result<(), String> {
    let path = settings_path(&app)?;
    let content = serde_json::to_string_pretty(&settings)
        .map_err(|e| format!("failed to serialize settings: {e}"))?;
    fs::write(&path, &content).map_err(|e| format!("failed to write settings: {e}"))?;
    Ok(())
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_dialog::init())
        .manage(AppState {
            repo_path: Mutex::new(None),
        })
        .invoke_handler(tauri::generate_handler![
            open_repository,
            get_repo_path,
            get_recent_repos,
            switch_repository,
            get_git_status,
            stage_file,
            unstage_file,
            commit_changes,
            get_file_diff,
            get_git_log,
            get_branches,
            get_commit_diff,
            get_binary_image_preview,
            stage_patch,
            rename_branch,
            delete_branch,
            merge_branch,
            create_branch,
            checkout_branch,
            checkout_remote_branch,
            git_fetch,
            get_ahead_behind,
            git_push,
            git_pull,
            pin_branch,
            unpin_branch,
            get_pinned_branches,
            create_tag,
            stash_save,
            stash_list,
            stash_apply,
            stash_drop,
            stash_file,
            get_staged_diff,
            get_git_config,
            set_git_config,
            load_settings,
            save_settings,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
