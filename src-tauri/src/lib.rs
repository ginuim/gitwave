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

fn run_git(repo: &str, args: &[&str]) -> Result<String, String> {
    let mut cmd = Command::new("git");
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
        let is_head = head_targets.iter().any(|t| *t == display_name);
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
    Ok(path_str)
}

#[tauri::command]
fn get_repo_path(state: State<'_, AppState>) -> Result<Option<String>, String> {
    let guard = state.repo_path.lock().map_err(|_| "state lock poisoned")?;
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

#[tauri::command]
fn get_file_diff(state: State<'_, AppState>, path: String, is_staged: bool) -> Result<String, String> {
    let repo = require_repo(&state)?;
    let p = normalize_path_for_git(&path);
    let raw = if is_staged {
        run_git(&repo, &["diff", "--cached", "--", &p])?
    } else {
        run_git(&repo, &["diff", "--", &p])?
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
            rename_branch,
            delete_branch,
            merge_branch,
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
