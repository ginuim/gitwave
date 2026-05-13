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
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
