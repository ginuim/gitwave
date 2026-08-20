use base64::{engine::general_purpose::STANDARD, Engine};
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use tauri::{Manager, State};
use tauri_plugin_dialog::DialogExt;
use tokio::sync::{oneshot, Semaphore};

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
pub struct BranchTip {
    pub name: String,
    pub hash: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CommitLog {
    pub hash: String,
    pub parents: Vec<String>,
    pub refs: Vec<String>,
    pub author: String,
    pub date: String,
    pub message: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CommitLogPage {
    pub commits: Vec<CommitLog>,
    pub has_more: bool,
}

fn parse_commit_refs(decorations: &str) -> Vec<String> {
    let trimmed = decorations.trim();
    if trimmed.is_empty() {
        return Vec::new();
    }
    trimmed
        .split(", ")
        .filter_map(|part| {
            let part = part.trim();
            if part.is_empty() {
                return None;
            }
            if let Some(tag) = part.strip_prefix("tag: ") {
                return Some(tag.to_string());
            }
            if let Some((_head, branch)) = part.split_once(" -> ") {
                return Some(branch.trim().to_string());
            }
            Some(part.to_string())
        })
        .collect()
}

fn parse_commit_parents(parents: &str) -> Vec<String> {
    let trimmed = parents.trim();
    if trimmed.is_empty() {
        return Vec::new();
    }
    trimmed
        .split_whitespace()
        .map(|hash| hash.to_string())
        .collect()
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StashEntry {
    pub index: usize,
    pub message: String,
    pub branch: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WorktreeState {
    pub has_changes: bool,
    pub in_merge: bool,
    pub in_rebase: bool,
    pub in_cherry_pick: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum OperationKind {
    None,
    Merge,
    Rebase,
    CherryPick,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OperationState {
    pub kind: OperationKind,
    pub conflicted_files: Vec<String>,
    pub has_conflicts: bool,
    pub can_continue: bool,
    pub can_abort: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SubtreeInfo {
    pub prefix: String,
    pub split_commit: Option<String>,
    pub pending_changes: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SubmoduleInfo {
    pub path: String,
    pub head: String,
    pub ref_name: Option<String>,
    pub status: String,
}

pub struct AppState {
    repo_path: Mutex<Option<String>>,
    git_gate: Arc<Semaphore>,
}

fn file_path_to_string(fp: tauri_plugin_dialog::FilePath) -> String {
    fp.to_string()
}

/// Git 在 `core.quotepath=true` 时会把非 ASCII 路径打成 `"\344\275\240..."`，需还原为真实路径。
fn unquote_git_path(path: &str) -> String {
    let path = path.trim();
    if path.len() < 2 || !path.starts_with('"') || !path.ends_with('"') {
        return path.to_string();
    }
    let inner = &path[1..path.len() - 1];
    let mut out = String::new();
    let bytes = inner.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'\\' && i + 1 < bytes.len() {
            i += 1;
            match bytes[i] {
                b'"' => out.push('"'),
                b'\\' => out.push('\\'),
                b'n' => out.push('\n'),
                b't' => out.push('\t'),
                b'0'..=b'7' => {
                    let start = i;
                    i += 1;
                    while i < bytes.len() && i - start < 3 && bytes[i].is_ascii_digit() {
                        i += 1;
                    }
                    let octal = std::str::from_utf8(&bytes[start..i]).unwrap_or("0");
                    if let Ok(v) = u32::from_str_radix(octal, 8) {
                        if let Some(ch) = char::from_u32(v) {
                            out.push(ch);
                        }
                    }
                    continue;
                }
                c => out.push(c as char),
            }
            i += 1;
        } else {
            out.push(bytes[i] as char);
            i += 1;
        }
    }
    out
}

fn normalize_path_for_git(path: &str) -> String {
    let path = unquote_git_path(path);
    #[cfg(windows)]
    {
        path.replace('\\', "/")
    }
    #[cfg(not(windows))]
    {
        path
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

fn log_git_perf(repo: &str, args: &[&str], elapsed: Duration, success: bool, output_bytes: usize) {
    eprintln!(
        "[gitwave:git] success={} elapsed_ms={} bytes={} repo={} command=\"git {}\"",
        success,
        elapsed.as_millis(),
        output_bytes,
        repo,
        args.join(" ")
    );
}

/// 尽力开启能显著加速 Windows 上 `git status` 的仓库本地配置。失败静默忽略。
fn ensure_fast_status_config(repo: &str) {
    let _ = run_git(repo, &["config", "core.fsmonitor", "true"]);
    let _ = run_git(repo, &["config", "core.untrackedCache", "true"]);
}

fn run_git(repo: &str, args: &[&str]) -> Result<String, String> {
    run_git_with_config(repo, &[], args)
}

/// 与 `run_git` 相同，但可在子命令前注入 `-c key=value`（如关闭 quotepath）。
fn run_git_with_config(
    repo: &str,
    config: &[(&str, &str)],
    args: &[&str],
) -> Result<String, String> {
    let mut cmd = Command::new("git");
    hide_git_child_console(&mut cmd);
    cmd.current_dir(repo);
    for (key, value) in config {
        cmd.arg("-c").arg(format!("{key}={value}"));
    }
    cmd.args(args);
    set_git_utf8_env(&mut cmd);
    let started = Instant::now();
    let output = cmd
        .output()
        .map_err(|e| format!("failed to spawn git: {e}"))?;
    let output_bytes = output.stdout.len() + output.stderr.len();
    log_git_perf(
        repo,
        args,
        started.elapsed(),
        output.status.success(),
        output_bytes,
    );
    if !output.status.success() {
        return Err(String::from_utf8_lossy(&output.stderr).into_owned());
    }
    Ok(String::from_utf8_lossy(&output.stdout).into_owned())
}

fn run_git_bytes(repo: &str, args: &[&str]) -> Result<Vec<u8>, String> {
    run_git_bytes_with_config(repo, &[], args)
}

fn run_git_bytes_with_config(
    repo: &str,
    config: &[(&str, &str)],
    args: &[&str],
) -> Result<Vec<u8>, String> {
    let mut cmd = Command::new("git");
    hide_git_child_console(&mut cmd);
    cmd.current_dir(repo);
    for (key, value) in config {
        cmd.arg("-c").arg(format!("{key}={value}"));
    }
    cmd.args(args);
    set_git_utf8_env(&mut cmd);
    let started = Instant::now();
    let output = cmd
        .output()
        .map_err(|e| format!("failed to spawn git: {e}"))?;
    let output_bytes = output.stdout.len() + output.stderr.len();
    log_git_perf(
        repo,
        args,
        started.elapsed(),
        output.status.success(),
        output_bytes,
    );
    if !output.status.success() {
        return Err(String::from_utf8_lossy(&output.stderr).into_owned());
    }
    Ok(output.stdout)
}

/// 运行 git diff 类命令，容忍退出码 1（表示「有差异」），
/// 与普通 `git diff` 不同，`--no-index` 模式有差异时退出码为 1。
fn run_git_diff(repo: &str, args: &[&str]) -> Result<String, String> {
    let mut cmd = Command::new("git");
    hide_git_child_console(&mut cmd);
    cmd.current_dir(repo);
    cmd.args(args);
    set_git_utf8_env(&mut cmd);
    let started = Instant::now();
    let output = cmd
        .output()
        .map_err(|e| format!("failed to spawn git: {e}"))?;
    let code = output.status.code().unwrap_or(0);
    let success = code == 0 || code == 1;
    let output_bytes = output.stdout.len() + output.stderr.len();
    log_git_perf(repo, args, started.elapsed(), success, output_bytes);
    if !success {
        return Err(String::from_utf8_lossy(&output.stderr).into_owned());
    }
    Ok(String::from_utf8_lossy(&output.stdout).into_owned())
}

fn run_git_with_stdin(repo: &str, args: &[&str], input: &[u8]) -> Result<String, String> {
    let mut cmd = Command::new("git");
    hide_git_child_console(&mut cmd);
    cmd.current_dir(repo);
    cmd.args(args);
    set_git_utf8_env(&mut cmd);
    cmd.stdin(Stdio::piped());
    cmd.stdout(Stdio::piped());
    cmd.stderr(Stdio::piped());

    let mut child = cmd
        .spawn()
        .map_err(|e| format!("failed to spawn git: {e}"))?;
    {
        let stdin = child
            .stdin
            .as_mut()
            .ok_or_else(|| "failed to open git stdin".to_string())?;
        stdin
            .write_all(input)
            .map_err(|e| format!("failed to write patch: {e}"))?;
    }
    let started = Instant::now();
    let output = child
        .wait_with_output()
        .map_err(|e| format!("failed to wait for git: {e}"))?;
    let output_bytes = output.stdout.len() + output.stderr.len();
    log_git_perf(
        repo,
        args,
        started.elapsed(),
        output.status.success(),
        output_bytes,
    );
    if !output.status.success() {
        return Err(String::from_utf8_lossy(&output.stderr).into_owned());
    }
    Ok(String::from_utf8_lossy(&output.stdout).into_owned())
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
    let started = Instant::now();
    let out = cmd
        .output()
        .map_err(|e| format!("failed to spawn git: {e}"))?;
    let args = ["rev-parse", "-q", "--verify", rev_path];
    let output_bytes = out.stdout.len() + out.stderr.len();
    log_git_perf(
        repo,
        &args,
        started.elapsed(),
        out.status.success(),
        output_bytes,
    );
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
    Ok(Some(
        fs::read(&file_canon).map_err(|e| format!("read file: {e}"))?,
    ))
}

fn worktree_file_path(repo: &str, rel: &str) -> Result<PathBuf, String> {
    ensure_safe_repo_relative_path(rel)?;
    let root = Path::new(repo);
    let path = root.join(rel);
    let root_canon = root
        .canonicalize()
        .map_err(|e| format!("failed to canonicalize repo root: {e}"))?;
    if path.exists() {
        let file_canon = path
            .canonicalize()
            .map_err(|e| format!("failed to canonicalize file path: {e}"))?;
        if !file_canon.starts_with(&root_canon) {
            return Err("path escapes repository".to_string());
        }
        return Ok(file_canon);
    }
    let parent = path
        .parent()
        .ok_or_else(|| "invalid file path".to_string())?
        .canonicalize()
        .map_err(|e| format!("failed to canonicalize parent path: {e}"))?;
    if !parent.starts_with(&root_canon) {
        return Err("path escapes repository".to_string());
    }
    Ok(path)
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
            let new = read_worktree_file_bytes(&repo, &rel)?.map(|b| bytes_to_data_url(&b, mime));
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
            let hash =
                commit_hash.ok_or_else(|| "commitHash required for commit preview".to_string())?;
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

fn git_state_path_exists(repo: &str, name: &str) -> bool {
    let path = match run_git(repo, &["rev-parse", "--git-path", name]) {
        Ok(p) => p,
        Err(_) => return false,
    };
    let path = path.trim();
    !path.is_empty() && Path::new(path).exists()
}

fn is_in_merge(repo: &str) -> bool {
    run_git(repo, &["rev-parse", "-q", "--verify", "MERGE_HEAD"]).is_ok()
}

fn is_in_rebase(repo: &str) -> bool {
    git_state_path_exists(repo, "rebase-merge") || git_state_path_exists(repo, "rebase-apply")
}

fn is_in_cherry_pick(repo: &str) -> bool {
    run_git(repo, &["rev-parse", "-q", "--verify", "CHERRY_PICK_HEAD"]).is_ok()
}

fn current_operation_kind(repo: &str) -> OperationKind {
    if is_in_rebase(repo) {
        OperationKind::Rebase
    } else if is_in_cherry_pick(repo) {
        OperationKind::CherryPick
    } else if is_in_merge(repo) {
        OperationKind::Merge
    } else {
        OperationKind::None
    }
}

fn conflicted_files(repo: &str) -> Result<Vec<String>, String> {
    let raw = run_git_with_config(
        repo,
        &[("core.quotepath", "false")],
        &["diff", "--name-only", "--diff-filter=U"],
    )?;
    Ok(raw
        .lines()
        .map(|line| normalize_path_for_git(line.trim()))
        .filter(|line| !line.is_empty())
        .collect())
}

fn operation_state(repo: &str) -> Result<OperationState, String> {
    let kind = current_operation_kind(repo);
    let conflicted_files = if kind == OperationKind::None {
        Vec::new()
    } else {
        conflicted_files(repo)?
    };
    let has_conflicts = !conflicted_files.is_empty();
    let active = kind != OperationKind::None;
    Ok(OperationState {
        kind,
        conflicted_files,
        has_conflicts,
        can_continue: active && !has_conflicts,
        can_abort: active,
    })
}

fn has_worktree_changes(repo: &str) -> Result<bool, String> {
    let raw = run_git_with_config(
        repo,
        &[("core.quotepath", "false")],
        &["--no-optional-locks", "status", "--porcelain"],
    )?;
    Ok(!raw.trim().is_empty())
}

fn discard_all_changes(repo: &str) -> Result<(), String> {
    if is_in_rebase(repo) {
        run_git(repo, &["rebase", "--abort"])?;
    }
    if is_in_merge(repo) {
        run_git(repo, &["merge", "--abort"])?;
    }
    if is_in_cherry_pick(repo) {
        run_git(repo, &["cherry-pick", "--abort"])?;
    }
    run_git(repo, &["reset", "--hard"])?;
    run_git(repo, &["clean", "-fd"])?;
    Ok(())
}

fn prepare_checkout(repo: &str, target: &str, mode: &str) -> Result<(), String> {
    match mode {
        "normal" => Ok(()),
        "stash" => {
            let msg = format!("WIP before switching to {target}");
            run_git(repo, &["stash", "push", "-u", "-m", &msg]).map(|_| ())
        }
        "discard" => discard_all_changes(repo),
        other => Err(format!("unknown checkout mode: {other}")),
    }
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
    let json = serde_json::to_string(repos).map_err(|e| format!("serialization error: {e}"))?;
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
    let json = serde_json::to_string(branches).map_err(|e| format!("serialization error: {e}"))?;
    fs::write(&path, &json).map_err(|e| format!("write error: {e}"))?;
    Ok(())
}

fn parse_porcelain_path(rest: &str) -> String {
    let rest = rest.trim_start();
    let path = if let Some(pos) = rest.rfind(" -> ") {
        rest[pos + 4..].trim()
    } else {
        rest.trim()
    };
    unquote_git_path(path)
}

fn is_submodule_path(repo: &str, rel: &str) -> bool {
    run_git(repo, &["ls-files", "-s", "--", rel])
        .ok()
        .and_then(|out| out.lines().next().map(|line| line.starts_with("160000 ")))
        .unwrap_or(false)
}

fn push_status_entries(out: &mut Vec<FileStatus>, path: String, index: char, worktree: char) {
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

fn parse_v2_xy(xy: &str) -> Option<(char, char)> {
    let mut chars = xy.chars();
    let index = match chars.next()? {
        '.' => ' ',
        ch => ch,
    };
    let worktree = match chars.next()? {
        '.' => ' ',
        ch => ch,
    };
    Some((index, worktree))
}

fn parse_v2_path_record(record: &str, path_field_index: usize) -> Option<(String, char, char)> {
    let mut parts = record.splitn(3, ' ');
    let _kind = parts.next()?;
    let (index, worktree) = parse_v2_xy(parts.next()?)?;
    let mut rest = parts.next()?;
    for _ in 2..path_field_index {
        let (_, next) = rest.split_once(' ')?;
        rest = next;
    }
    let path = rest;
    if path.is_empty() {
        return None;
    }
    Some((normalize_path_for_git(path), index, worktree))
}

fn parse_git_status_porcelain_v2_z(raw: &[u8]) -> Vec<FileStatus> {
    let mut out = Vec::new();
    let mut records = raw.split(|b| *b == 0);
    while let Some(record) = records.next() {
        if record.is_empty() {
            continue;
        }
        let text = String::from_utf8_lossy(record);
        let text = text.trim_end_matches('\n');
        if text.is_empty() || text.starts_with('#') {
            continue;
        }
        if let Some(path) = text.strip_prefix("? ") {
            let path = normalize_path_for_git(path);
            if !path.is_empty() {
                push_status_entries(&mut out, path, '?', '?');
            }
            continue;
        }
        if text.starts_with("! ") {
            continue;
        }
        let parsed = if text.starts_with("1 ") {
            parse_v2_path_record(text, 8)
        } else if text.starts_with("2 ") {
            let parsed = parse_v2_path_record(text, 9);
            let _orig_path = records.next();
            parsed
        } else if text.starts_with("u ") {
            parse_v2_path_record(text, 10)
        } else {
            None
        };
        if let Some((path, index, worktree)) = parsed {
            push_status_entries(&mut out, path, index, worktree);
        }
    }
    out
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

fn open_repository_at(app: &tauri::AppHandle, path_str: String) -> Result<String, String> {
    let root = PathBuf::from(&path_str);
    if !root.is_dir() {
        return Err(format!("not a directory: {path_str}"));
    }
    if !is_git_dir(&root) {
        return Err(format!("not a git repository (no .git at): {path_str}"));
    }
    {
        let mut recent = load_recent_repos(app);
        recent.retain(|r| r != &path_str);
        recent.insert(0, path_str.clone());
        recent.truncate(10);
        save_recent_repos(app, &recent)?;

        let state = app.state::<AppState>();
        let mut guard = state.repo_path.lock().map_err(|_| "state lock poisoned")?;
        *guard = Some(path_str.clone());
    }
    save_last_repo(app, &path_str)?;
    ensure_fast_status_config(&path_str);
    Ok(path_str)
}

#[tauri::command]
async fn open_repository(app: tauri::AppHandle) -> Result<String, String> {
    let (tx, rx) = oneshot::channel();

    app.dialog().file().pick_folder(move |file_path| {
        let _ = tx.send(file_path);
    });

    let picked = rx
        .await
        .map_err(|_| "dialog cancelled".to_string())?
        .ok_or_else(|| "dialog cancelled".to_string())?;

    open_repository_at(&app, file_path_to_string(picked))
}

#[tauri::command]
fn open_repository_at_path(app: tauri::AppHandle, path: String) -> Result<String, String> {
    open_repository_at(&app, path)
}

#[tauri::command]
fn get_repo_path(
    state: State<'_, AppState>,
    app: tauri::AppHandle,
) -> Result<Option<String>, String> {
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
async fn clone_repository(url: String, target_dir: String) -> Result<String, String> {
    let target = PathBuf::from(&target_dir);

    // Create parent directory if it doesn't exist
    if let Some(parent) = target.parent() {
        fs::create_dir_all(parent).map_err(|e| format!("failed to create directory: {e}"))?;
    }

    // Run git clone
    let result = tokio::task::spawn_blocking(move || {
        let mut cmd = Command::new("git");
        hide_git_child_console(&mut cmd);
        cmd.arg("clone");
        cmd.arg(&url);
        cmd.arg(&target_dir);
        set_git_utf8_env(&mut cmd);

        let output = cmd
            .output()
            .map_err(|e| format!("failed to spawn git clone: {e}"))?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            return Err(format!("git clone failed: {stderr}"));
        }

        Ok(target_dir)
    })
    .await
    .map_err(|e| format!("clone task failed: {e}"))?;

    result
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
    ensure_fast_status_config(&path);
    Ok(path)
}

#[tauri::command]
async fn get_git_status(state: State<'_, AppState>) -> Result<Vec<FileStatus>, String> {
    let repo = require_repo(&state)?;
    let gate = Arc::clone(&state.git_gate);
    let _permit = gate
        .acquire_owned()
        .await
        .map_err(|_| "git task gate closed".to_string())?;
    tokio::task::spawn_blocking(move || {
        let raw = run_git_bytes_with_config(
            &repo,
            &[("core.quotepath", "false")],
            &[
                "--no-optional-locks",
                "status",
                "--porcelain=v2",
                "-z",
                "--branch",
            ],
        )?;
        Ok(parse_git_status_porcelain_v2_z(&raw))
    })
    .await
    .map_err(|e| format!("status task failed: {e}"))?
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

/// 丢弃变更：`is_staged` 为 true 时还原索引与工作区到 HEAD；否则仅丢弃工作区未 stage 部分。
/// 未跟踪文件（`??`）使用 `git clean -f`。
#[tauri::command]
fn revert_file(state: State<'_, AppState>, path: String, is_staged: bool) -> Result<(), String> {
    let repo = require_repo(&state)?;
    let p = normalize_path_for_git(&path);
    let status_raw = run_git_with_config(
        &repo,
        &[("core.quotepath", "false")],
        &["status", "--porcelain", "--", &p],
    )?;
    let first = status_raw.lines().next().unwrap_or("").trim();
    if first.len() >= 2 {
        let index = first.as_bytes()[0] as char;
        let worktree = first.as_bytes()[1] as char;
        if index == '?' && worktree == '?' {
            run_git(&repo, &["clean", "-f", "--", &p])?;
            return Ok(());
        }
    }
    if !is_staged && is_submodule_path(&repo, &p) {
        let sub = Path::new(&repo).join(&p);
        if sub.is_dir() {
            let sub_str = sub
                .to_str()
                .ok_or_else(|| "invalid submodule path".to_string())?;
            run_git(sub_str, &["restore", "--worktree", "."])?;
            return Ok(());
        }
    }
    if is_staged {
        run_git(
            &repo,
            &[
                "restore",
                "--source=HEAD",
                "--staged",
                "--worktree",
                "--",
                &p,
            ],
        )?;
    } else {
        run_git(&repo, &["restore", "--worktree", "--", &p])?;
    }
    Ok(())
}

/// 从磁盘删除文件：未跟踪文件用 `git clean -f`；已跟踪文件用 `git rm -f`（删除并 stage）。
#[tauri::command]
fn delete_file(state: State<'_, AppState>, path: String, _is_staged: bool) -> Result<(), String> {
    let repo = require_repo(&state)?;
    let p = normalize_path_for_git(&path);
    let status_raw = run_git_with_config(
        &repo,
        &[("core.quotepath", "false")],
        &["status", "--porcelain", "--", &p],
    )?;
    let first = status_raw.lines().next().unwrap_or("").trim();
    if first.len() >= 2 {
        let index = first.as_bytes()[0] as char;
        let worktree = first.as_bytes()[1] as char;
        if index == '?' && worktree == '?' {
            run_git(&repo, &["clean", "-f", "--", &p])?;
            return Ok(());
        }
    }
    run_git(&repo, &["rm", "-f", "--", &p])?;
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
fn amend_last_commit(state: State<'_, AppState>, message: String) -> Result<(), String> {
    let repo = require_repo(&state)?;
    if message.trim().is_empty() {
        return Err("empty commit message".to_string());
    }
    run_git(&repo, &["commit", "--amend", "-m", &message])?;
    Ok(())
}

#[tauri::command]
fn soft_reset_last_commit(state: State<'_, AppState>) -> Result<String, String> {
    let repo = require_repo(&state)?;
    let output = run_git(&repo, &["reset", "--soft", "HEAD~1"])?;
    Ok(if output.is_empty() {
        "ok".to_string()
    } else {
        output
    })
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

fn file_diff_for_repo(repo: &str, path: &str, is_staged: bool) -> Result<String, String> {
    let p = normalize_path_for_git(&path);
    ensure_safe_repo_relative_path(&p)?;
    let raw = if is_staged {
        run_git(repo, &["diff", "--cached", "--", &p])?
    } else {
        let diff = run_git(repo, &["diff", "--", &p])?;
        if !diff.is_empty() {
            diff
        } else if !is_tracked_in_index(repo, &p)? {
            let worktree = Path::new(repo).join(&p);
            if worktree.is_file() {
                run_git_diff(repo, &["diff", "--no-index", "--", GIT_NULL_PATH, &p])?
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
async fn get_file_diff(
    state: State<'_, AppState>,
    path: String,
    is_staged: bool,
) -> Result<String, String> {
    let repo = require_repo(&state)?;
    let gate = Arc::clone(&state.git_gate);
    let _permit = gate
        .acquire_owned()
        .await
        .map_err(|_| "git task gate closed".to_string())?;
    tokio::task::spawn_blocking(move || file_diff_for_repo(&repo, &path, is_staged))
        .await
        .map_err(|e| format!("diff task failed: {e}"))?
}

fn git_log_for_repo(
    repo: &str,
    all: Option<bool>,
    skip: Option<usize>,
    limit: Option<usize>,
) -> Result<CommitLogPage, String> {
    let skip = skip.unwrap_or(0);
    let limit = limit.unwrap_or(50).max(1);
    let fetch = limit.saturating_add(1);
    let format_str = "%H%x00%P%x00%D%x00%an%x00%ad%x00%s";
    let pretty = format!("--pretty=format:{format_str}");
    let skip_arg = format!("--skip={skip}");
    let limit_arg = format!("-n{fetch}");
    let mut args = vec!["log", &skip_arg, &limit_arg, &pretty, "--date=iso"];
    if all.unwrap_or(false) {
        args.push("--all");
    }
    let raw = run_git(repo, &args)?;
    let mut logs = Vec::new();
    for line in raw.split('\n') {
        if line.is_empty() {
            continue;
        }
        let mut parts = line.split('\0');
        let hash = parts.next().unwrap_or("").to_string();
        let parents = parse_commit_parents(parts.next().unwrap_or(""));
        let refs = parse_commit_refs(parts.next().unwrap_or(""));
        let author = parts.next().unwrap_or("").to_string();
        let date = parts.next().unwrap_or("").to_string();
        let message = parts.next().unwrap_or("").to_string();
        logs.push(CommitLog {
            hash,
            parents,
            refs,
            author,
            date,
            message,
        });
    }
    let has_more = logs.len() > limit;
    if has_more {
        logs.truncate(limit);
    }
    Ok(CommitLogPage {
        commits: logs,
        has_more,
    })
}

#[tauri::command]
async fn get_git_log(
    state: State<'_, AppState>,
    all: Option<bool>,
    skip: Option<usize>,
    limit: Option<usize>,
) -> Result<CommitLogPage, String> {
    let repo = require_repo(&state)?;
    let gate = Arc::clone(&state.git_gate);
    let _permit = gate
        .acquire_owned()
        .await
        .map_err(|_| "git task gate closed".to_string())?;
    tokio::task::spawn_blocking(move || git_log_for_repo(&repo, all, skip, limit))
        .await
        .map_err(|e| format!("log task failed: {e}"))?
}

#[tauri::command(async)]
fn get_branches(state: State<'_, AppState>) -> Result<Vec<BranchInfo>, String> {
    let repo = require_repo(&state)?;
    let raw = run_git(&repo, &["branch", "-a"])?;
    Ok(parse_branches(&raw))
}

#[tauri::command(async)]
fn get_branch_tips(state: State<'_, AppState>) -> Result<Vec<BranchTip>, String> {
    let repo = require_repo(&state)?;
    let raw = run_git(
        &repo,
        &[
            "for-each-ref",
            "--format=%(refname:short)%00%(objectname)",
            "refs/heads",
            "refs/remotes",
        ],
    )?;
    let mut tips = Vec::new();
    for line in raw.lines() {
        if line.is_empty() {
            continue;
        }
        let mut parts = line.split('\0');
        let name = parts.next().unwrap_or("").trim().to_string();
        let hash = parts.next().unwrap_or("").trim().to_string();
        if name.is_empty() || hash.is_empty() {
            continue;
        }
        if name.ends_with("/HEAD") {
            continue;
        }
        tips.push(BranchTip { name, hash });
    }
    Ok(tips)
}

#[tauri::command]
fn get_commit_diff(state: State<'_, AppState>, hash: String) -> Result<String, String> {
    let repo = require_repo(&state)?;
    // Merge commits omit the patch in plain `git show`; -m --first-parent diffs vs mainline parent.
    run_git(&repo, &["show", "-m", "--first-parent", &hash])
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AheadBehind {
    pub ahead: usize,
    pub behind: usize,
    pub unpushed_hashes: Vec<String>,
}

#[tauri::command]
fn rename_branch(
    state: State<'_, AppState>,
    old_name: String,
    new_name: String,
) -> Result<String, String> {
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
fn create_branch_at(
    state: State<'_, AppState>,
    name: String,
    hash: String,
) -> Result<String, String> {
    let repo = require_repo(&state)?;
    run_git(&repo, &["branch", &name, &hash])
}

#[tauri::command]
fn get_worktree_state(state: State<'_, AppState>) -> Result<WorktreeState, String> {
    let repo = require_repo(&state)?;
    Ok(WorktreeState {
        has_changes: has_worktree_changes(&repo)?,
        in_merge: is_in_merge(&repo),
        in_rebase: is_in_rebase(&repo),
        in_cherry_pick: is_in_cherry_pick(&repo),
    })
}

#[tauri::command(async)]
fn get_operation_state(state: State<'_, AppState>) -> Result<OperationState, String> {
    let repo = require_repo(&state)?;
    operation_state(&repo)
}

#[tauri::command]
fn continue_operation(state: State<'_, AppState>) -> Result<String, String> {
    let repo = require_repo(&state)?;
    match current_operation_kind(&repo) {
        OperationKind::Rebase => run_git(&repo, &["rebase", "--continue"]),
        OperationKind::CherryPick => run_git(&repo, &["cherry-pick", "--continue"]),
        OperationKind::Merge => run_git(&repo, &["commit", "--no-edit"]),
        OperationKind::None => Err("no operation in progress".to_string()),
    }
}

#[tauri::command]
fn abort_operation(state: State<'_, AppState>) -> Result<String, String> {
    let repo = require_repo(&state)?;
    match current_operation_kind(&repo) {
        OperationKind::Rebase => run_git(&repo, &["rebase", "--abort"]),
        OperationKind::CherryPick => run_git(&repo, &["cherry-pick", "--abort"]),
        OperationKind::Merge => run_git(&repo, &["merge", "--abort"]),
        OperationKind::None => Err("no operation in progress".to_string()),
    }
}

#[tauri::command]
fn mark_file_resolved(state: State<'_, AppState>, path: String) -> Result<String, String> {
    let repo = require_repo(&state)?;
    let p = normalize_path_for_git(&path);
    ensure_safe_repo_relative_path(&p)?;
    run_git(&repo, &["add", "--", &p])
}

#[tauri::command]
fn read_working_file(state: State<'_, AppState>, path: String) -> Result<String, String> {
    let repo = require_repo(&state)?;
    let p = normalize_path_for_git(&path);
    let bytes =
        read_worktree_file_bytes(&repo, &p)?.ok_or_else(|| "file does not exist".to_string())?;
    String::from_utf8(bytes).map_err(|_| "file is not valid UTF-8".to_string())
}

#[tauri::command]
fn write_working_file(
    state: State<'_, AppState>,
    path: String,
    content: String,
) -> Result<String, String> {
    let repo = require_repo(&state)?;
    let p = normalize_path_for_git(&path);
    let file = worktree_file_path(&repo, &p)?;
    fs::write(&file, content).map_err(|e| format!("write file: {e}"))?;
    Ok("ok".to_string())
}

#[tauri::command]
fn cherry_pick_commit(state: State<'_, AppState>, hash: String) -> Result<String, String> {
    let repo = require_repo(&state)?;
    run_git(&repo, &["cherry-pick", &hash])
}

#[tauri::command]
fn revert_commit(state: State<'_, AppState>, hash: String) -> Result<String, String> {
    let repo = require_repo(&state)?;
    run_git(&repo, &["revert", &hash])
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
fn checkout_with_mode(
    state: State<'_, AppState>,
    target: String,
    mode: String,
    is_remote: bool,
) -> Result<String, String> {
    let repo = require_repo(&state)?;
    prepare_checkout(&repo, &target, &mode)?;
    if is_remote {
        let local = target.split('/').last().unwrap_or(&target);
        run_git(&repo, &["checkout", "-b", local, "--track", &target])
    } else {
        run_git(&repo, &["checkout", &target])
    }
}

#[tauri::command]
async fn git_fetch(state: State<'_, AppState>) -> Result<String, String> {
    let repo = require_repo(&state)?;
    let gate = Arc::clone(&state.git_gate);
    let _permit = gate
        .acquire_owned()
        .await
        .map_err(|_| "git task gate closed".to_string())?;
    let result = tokio::task::spawn_blocking(move || run_git(&repo, &["fetch"]))
        .await
        .map_err(|e| format!("task failed: {e}"))?;
    result.map(|s| if s.is_empty() { "ok".into() } else { s })
}

#[tauri::command]
fn stage_patch(state: State<'_, AppState>, patch: String) -> Result<String, String> {
    let repo = require_repo(&state)?;
    run_git_with_stdin(&repo, &["apply", "--cached", "-"], patch.as_bytes())
}

/// 从 unified diff 中解析 `+++ b/<path>` 路径（含 quotepath 引号路径）。
fn patch_target_path(patch: &str) -> Option<String> {
    for line in patch.lines() {
        let Some(rest) = line.strip_prefix("+++ ") else {
            continue;
        };
        let path = rest.trim();
        if path == "/dev/null" || path == "NUL" {
            continue;
        }
        let raw = path.strip_prefix("b/").unwrap_or(path);
        return Some(normalize_path_for_git(raw));
    }
    None
}

/// 取用于 hunk 定位的 diff 文本（未跟踪文件走 `--no-index`）。
fn repo_diff_for_hunk(repo: &str, path: &str, is_staged: bool) -> Result<String, String> {
    let p = normalize_path_for_git(path);
    if is_staged {
        return run_git(repo, &["diff", "--cached", "--", &p]);
    }
    let diff = run_git(repo, &["diff", "--", &p])?;
    if !diff.is_empty() {
        return Ok(diff);
    }
    if !is_tracked_in_index(repo, &p)? {
        let worktree = Path::new(repo).join(&p);
        if worktree.is_file() {
            return run_git_diff(repo, &["diff", "--no-index", "--", GIT_NULL_PATH, &p]);
        }
    }
    Ok(diff)
}

/// 指定 @@ 头是否仍存在于 diff 中（精确匹配，避免误判相邻 hunk）。
fn hunk_still_in_diff(
    repo: &str,
    path: &str,
    is_staged: bool,
    header: &str,
) -> Result<bool, String> {
    let diff = repo_diff_for_hunk(repo, path, is_staged)?;
    Ok(diff.lines().any(|line| line == header))
}

fn patch_hunk_header(patch: &str) -> Option<String> {
    patch
        .lines()
        .find(|l| l.starts_with("@@ "))
        .map(str::to_string)
}

/// 从完整 diff 文本中按 @@ 头取出单个 hunk（含 @@ 行）。
/// 先尝试精确匹配 header 行；若失败（行号变了），按旧行号在 ±10 行范围内模糊匹配。
fn extract_hunk_from_diff(diff: &str, header: &str) -> Option<String> {
    let lines: Vec<&str> = diff.lines().collect();

    // 1. 精确匹配
    if let Some(start) = lines.iter().position(|l| l == &header) {
        let mut end = lines.len();
        for (i, line) in lines.iter().enumerate().skip(start + 1) {
            if line.starts_with("@@ ") {
                end = i;
                break;
            }
        }
        let body = lines[start..end].join("\n");
        if !body.is_empty() {
            return Some(format!("{body}\n"));
        }
    }

    // 2. 模糊匹配：解析旧行号，在 ±10 行范围内找最接近的 @@ 头
    let old_line = header
        .strip_prefix("@@ -")
        .and_then(|s| s.split(',').next())
        .and_then(|s| s.parse::<i32>().ok())?;

    let mut best: Option<(usize, i32)> = None;
    for (i, line) in lines.iter().enumerate() {
        if !line.starts_with("@@ ") {
            continue;
        }
        if let Some(lo) = line
            .strip_prefix("@@ -")
            .and_then(|s| s.split(',').next())
            .and_then(|s| s.parse::<i32>().ok())
        {
            let dist = (lo - old_line).abs();
            if dist <= 10 && best.map_or(true, |(_, d)| dist < d) {
                best = Some((i, dist));
            }
        }
    }
    let start = best?.0;
    let mut end = lines.len();
    for (i, line) in lines.iter().enumerate().skip(start + 1) {
        if line.starts_with("@@ ") {
            end = i;
            break;
        }
    }
    let body = lines[start..end].join("\n");
    if body.is_empty() {
        None
    } else {
        Some(format!("{body}\n"))
    }
}

/// 从 `git diff` 输出中取出文件头（diff --git … 到首个 @@ 之前）。
fn diff_file_prefix(diff: &str) -> Option<String> {
    let lines: Vec<&str> = diff.lines().collect();
    let start = lines.iter().position(|l| l.starts_with("diff --git "))?;
    let hunk_at = lines.iter().position(|l| l.starts_with("@@ "))?;
    if hunk_at <= start {
        return None;
    }
    Some(lines[start..hunk_at].join("\n"))
}

fn build_apply_args(is_staged: bool, three_way: bool) -> Vec<String> {
    let mut args = vec!["apply".to_string(), "-R".to_string()];
    if is_staged {
        args.push("--cached".to_string());
    }
    if three_way {
        args.push("--3way".to_string());
    }
    args.extend(
        ["--whitespace=nowarn", "--recount", "--inaccurate-eof", "-"]
            .iter()
            .map(|s| s.to_string()),
    );
    args
}

fn try_apply_patch(
    repo: &str,
    patch: &[u8],
    is_staged: bool,
    three_way: bool,
) -> Result<(), String> {
    let args = build_apply_args(is_staged, three_way);
    let arg_refs: Vec<&str> = args.iter().map(String::as_str).collect();
    run_git_with_stdin(repo, &arg_refs, patch).map(|_| ())
}

/// 用当前仓库里重新生成的 diff 拼出 patch（避免 UI 中的 patch 与磁盘状态不一致）。
fn refresh_patch_from_repo(
    repo: &str,
    path: &str,
    is_staged: bool,
    header: &str,
) -> Result<Vec<u8>, String> {
    let p = normalize_path_for_git(path);
    let diff = repo_diff_for_hunk(repo, &p, is_staged)?;
    let prefix =
        diff_file_prefix(&diff).ok_or_else(|| "hunk not found in fresh diff".to_string())?;
    let hunk = extract_hunk_from_diff(&diff, header)
        .ok_or_else(|| "hunk not found in fresh diff".to_string())?;
    let mut patch = format!("{prefix}\n{hunk}").into_bytes();
    if !patch.ends_with(b"\n") {
        patch.push(b'\n');
    }
    Ok(patch)
}

/// 反向 apply；若 patch 的 @@ 头与仓库 diff 一致，则要求回退后该头消失。
fn try_revert_hunk(
    repo: &str,
    patch: &[u8],
    path: &str,
    is_staged: bool,
    header: &str,
    three_way: bool,
) -> Result<(), String> {
    let header_in_repo = hunk_still_in_diff(repo, path, is_staged, header)?;
    try_apply_patch(repo, patch, is_staged, three_way)?;
    if header_in_repo && hunk_still_in_diff(repo, path, is_staged, header)? {
        return Err("patch applied but hunk still present".to_string());
    }
    Ok(())
}

/// 反向应用 patch 以丢弃变更：`is_staged` 为 true 时作用于索引，否则作用于工作区。
#[tauri::command]
fn revert_patch(
    state: State<'_, AppState>,
    patch: String,
    is_staged: bool,
) -> Result<String, String> {
    let repo = require_repo(&state)?;
    let header =
        patch_hunk_header(&patch).ok_or_else(|| "patch has no @@ hunk header".to_string())?;
    let path = patch_target_path(&patch).ok_or_else(|| "patch has no file path".to_string())?;

    let mut input = patch.into_bytes();
    if !input.ends_with(b"\n") {
        input.push(b'\n');
    }

    let mut last_err = String::new();

    // 优先用仓库实时 diff 拼 patch，避免 UI 与磁盘不一致
    if let Ok(fresh) = refresh_patch_from_repo(&repo, &path, is_staged, &header) {
        for three_way in [false, true] {
            match try_revert_hunk(&repo, &fresh, &path, is_staged, &header, three_way) {
                Ok(()) => return Ok(String::new()),
                Err(e) => last_err = e,
            }
        }
    }

    for three_way in [false, true] {
        match try_revert_hunk(&repo, &input, &path, is_staged, &header, three_way) {
            Ok(()) => return Ok(String::new()),
            Err(e) => last_err = e,
        }
    }

    if !hunk_still_in_diff(&repo, &path, is_staged, &header)? {
        return Ok(String::new());
    }

    Err(if last_err.is_empty() {
        "failed to revert hunk".to_string()
    } else {
        last_err
    })
}

fn parse_ahead_behind(repo: &str, range: &str) -> Result<(usize, usize), String> {
    let output = run_git(repo, &["rev-list", "--count", "--left-right", range])?;
    let trimmed = output.trim();
    let parts: Vec<&str> = trimmed.split('\t').collect();
    let behind = parts.first().unwrap_or(&"0").parse().unwrap_or(0);
    let ahead = parts.get(1).copied().unwrap_or("0").parse().unwrap_or(0);
    Ok((ahead, behind))
}

fn compare_ref_for_branch(repo: &str, branch: &str) -> Result<String, String> {
    match run_git(
        repo,
        &[
            "rev-parse",
            "--abbrev-ref",
            "--symbolic-full-name",
            "@{upstream}",
        ],
    ) {
        Ok(upstream) => Ok(upstream.trim().to_string()),
        Err(_) => ahead_behind_compare_ref(repo, branch),
    }
}

fn list_unpushed_hashes(repo: &str, compare: &str) -> Result<Vec<String>, String> {
    let output = run_git(repo, &["rev-list", &format!("{compare}..HEAD")])?;
    Ok(output
        .lines()
        .filter(|line| !line.trim().is_empty())
        .map(str::to_string)
        .collect())
}

/// 无本地 upstream 时，用 push 会用的 remote 分支作对比基准。
fn ahead_behind_compare_ref(repo: &str, branch: &str) -> Result<String, String> {
    let remote = resolve_push_remote(repo, branch)?;
    let remote_branch = format!("{remote}/{branch}");
    if run_git(repo, &["rev-parse", "--verify", &remote_branch]).is_ok() {
        return Ok(remote_branch);
    }
    default_remote_branch(repo, &remote)
}

fn default_remote_branch(repo: &str, remote: &str) -> Result<String, String> {
    if let Ok(sym) = run_git(
        repo,
        &["symbolic-ref", &format!("refs/remotes/{remote}/HEAD")],
    ) {
        let sym = sym.trim();
        if let Some(short) = sym.strip_prefix("refs/remotes/") {
            return Ok(short.to_string());
        }
    }
    for name in ["main", "master", "develop"] {
        let candidate = format!("{remote}/{name}");
        if run_git(repo, &["rev-parse", "--verify", &candidate]).is_ok() {
            return Ok(candidate);
        }
    }
    Err(format!("无法确定 {remote} 的默认对比分支"))
}

fn ahead_behind_for_branch(repo: &str, branch: &str) -> Result<AheadBehind, String> {
    let compare = compare_ref_for_branch(repo, branch)?;
    let (ahead, behind) = parse_ahead_behind(repo, &format!("{compare}...HEAD"))?;
    let unpushed_hashes = if ahead == 0 {
        Vec::new()
    } else {
        list_unpushed_hashes(repo, &compare)?
    };
    Ok(AheadBehind {
        ahead,
        behind,
        unpushed_hashes,
    })
}

#[tauri::command(async)]
fn get_ahead_behind(state: State<'_, AppState>) -> Result<AheadBehind, String> {
    let repo = require_repo(&state)?;
    let branch = run_git(&repo, &["rev-parse", "--abbrev-ref", "HEAD"])?;
    let branch = branch.trim().to_string();
    if branch == "HEAD" {
        return Ok(AheadBehind {
            ahead: 0,
            behind: 0,
            unpushed_hashes: vec![],
        });
    }
    ahead_behind_for_branch(&repo, &branch)
}

fn has_upstream(repo: &str) -> bool {
    run_git(
        repo,
        &[
            "rev-parse",
            "--abbrev-ref",
            "--symbolic-full-name",
            "@{upstream}",
        ],
    )
    .is_ok()
}

/// 与 `git push` 失败提示一致：无 upstream 时用 `push --set-upstream <remote> <branch>`。
fn resolve_push_remote(repo: &str, branch: &str) -> Result<String, String> {
    for key in [
        format!("branch.{branch}.pushRemote"),
        format!("branch.{branch}.remote"),
        "remote.pushDefault".to_string(),
    ] {
        if let Ok(remote) = run_git(repo, &["config", "--get", &key]) {
            let remote = remote.trim();
            if !remote.is_empty() {
                return Ok(remote.to_string());
            }
        }
    }
    if run_git(repo, &["remote", "get-url", "origin"]).is_ok() {
        return Ok("origin".into());
    }
    let remotes = run_git(repo, &["remote"])?;
    remotes
        .lines()
        .map(str::trim)
        .find(|r| !r.is_empty())
        .map(|r| r.to_string())
        .ok_or_else(|| "未配置 git remote，无法 push".into())
}

fn git_push_repo(repo: &str) -> Result<String, String> {
    if has_upstream(repo) {
        return run_git(repo, &["push"]);
    }
    let branch = run_git(repo, &["rev-parse", "--abbrev-ref", "HEAD"])?
        .trim()
        .to_string();
    if branch == "HEAD" {
        return Err("当前为 detached HEAD，无法 push".into());
    }
    let remote = resolve_push_remote(repo, &branch)?;
    run_git(repo, &["push", "--set-upstream", &remote, &branch])
}

#[tauri::command]
async fn git_push(state: State<'_, AppState>) -> Result<String, String> {
    let repo = require_repo(&state)?;
    let gate = Arc::clone(&state.git_gate);
    let _permit = gate
        .acquire_owned()
        .await
        .map_err(|_| "git task gate closed".to_string())?;
    let result = tokio::task::spawn_blocking(move || git_push_repo(&repo))
        .await
        .map_err(|e| format!("task failed: {e}"))?;
    result.map(|s| if s.is_empty() { "ok".into() } else { s })
}

#[tauri::command]
async fn git_pull(state: State<'_, AppState>) -> Result<String, String> {
    let repo = require_repo(&state)?;
    let gate = Arc::clone(&state.git_gate);
    let _permit = gate
        .acquire_owned()
        .await
        .map_err(|_| "git task gate closed".to_string())?;
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

// === Subtree ===

fn path_under_prefix(path: &str, prefix: &str) -> bool {
    path == prefix || path.starts_with(&format!("{prefix}/"))
}

fn normalize_subtree_prefix(prefix: &str) -> String {
    prefix.trim().trim_matches('/').replace('\\', "/")
}

/// 从 git log 的 merge 提交信息中解析 subtree 前缀（log 按时间倒序，首次出现即最新）。
fn discover_subtree_prefixes(repo: &str) -> Result<Vec<(String, Option<String>)>, String> {
    let raw = run_git(
        repo,
        &[
            "log",
            "--all",
            "--grep=git-subtree-dir:",
            "-1000",
            "--format=%B%x00",
        ],
    )?;
    let mut seen: HashMap<String, Option<String>> = HashMap::new();
    for block in raw.split('\0') {
        let mut dir: Option<String> = None;
        let mut split: Option<String> = None;
        for line in block.lines() {
            if let Some(d) = line.strip_prefix("git-subtree-dir: ") {
                dir = Some(normalize_subtree_prefix(d));
            } else if let Some(s) = line.strip_prefix("git-subtree-split: ") {
                split = Some(s.trim().to_string());
            }
        }
        if let Some(d) = dir {
            if !d.is_empty() {
                seen.entry(d).or_insert(split);
            }
        }
    }
    let mut out: Vec<(String, Option<String>)> = seen.into_iter().collect();
    out.sort_by(|a, b| a.0.cmp(&b.0));
    Ok(out)
}

fn count_subtree_pending_changes(repo: &str, prefix: &str) -> Result<u32, String> {
    let raw = run_git_with_config(
        repo,
        &[("core.quotepath", "false")],
        &["status", "--porcelain"],
    )?;
    let mut paths = HashSet::new();
    for line in raw.lines() {
        let line = line.trim_end();
        if line.len() < 4 {
            continue;
        }
        let rest = line.get(3..).unwrap_or("");
        let path = parse_porcelain_path(rest);
        if !path.is_empty() && path_under_prefix(&path, prefix) {
            paths.insert(path);
        }
    }
    Ok(paths.len() as u32)
}

#[tauri::command(async)]
fn get_subtrees(state: State<'_, AppState>) -> Result<Vec<SubtreeInfo>, String> {
    let repo = require_repo(&state)?;
    let discovered = discover_subtree_prefixes(&repo)?;
    let mut out = Vec::with_capacity(discovered.len());
    for (prefix, split_commit) in discovered {
        let pending_changes = count_subtree_pending_changes(&repo, &prefix).unwrap_or(0);
        out.push(SubtreeInfo {
            prefix,
            split_commit,
            pending_changes,
        });
    }
    Ok(out)
}

fn run_subtree(
    repo: &str,
    action: &str,
    prefix: &str,
    remote: &str,
    branch: &str,
) -> Result<String, String> {
    let prefix = normalize_subtree_prefix(prefix);
    if prefix.is_empty() {
        return Err("subtree prefix 不能为空".into());
    }
    let remote = remote.trim();
    let branch = branch.trim();
    if remote.is_empty() {
        return Err("remote 不能为空".into());
    }
    if branch.is_empty() {
        return Err("branch 不能为空".into());
    }
    let prefix_flag = format!("--prefix={prefix}");
    run_git(repo, &["subtree", action, &prefix_flag, remote, branch])
}

#[tauri::command]
async fn subtree_pull(
    state: State<'_, AppState>,
    prefix: String,
    remote: String,
    branch: String,
) -> Result<String, String> {
    let repo = require_repo(&state)?;
    let result =
        tokio::task::spawn_blocking(move || run_subtree(&repo, "pull", &prefix, &remote, &branch))
            .await
            .map_err(|e| format!("task failed: {e}"))?;
    result.map(|s| if s.is_empty() { "ok".into() } else { s })
}

#[tauri::command]
async fn subtree_push(
    state: State<'_, AppState>,
    prefix: String,
    remote: String,
    branch: String,
) -> Result<String, String> {
    let repo = require_repo(&state)?;
    let result =
        tokio::task::spawn_blocking(move || run_subtree(&repo, "push", &prefix, &remote, &branch))
            .await
            .map_err(|e| format!("task failed: {e}"))?;
    result.map(|s| if s.is_empty() { "ok".into() } else { s })
}

fn submodule_status_name(prefix: char) -> &'static str {
    match prefix {
        '-' => "uninitialized",
        '+' => "modified",
        'U' => "conflict",
        _ => "clean",
    }
}

fn parse_submodule_status_line(line: &str) -> Option<SubmoduleInfo> {
    let mut chars = line.chars();
    let prefix = chars.next()?;
    let rest = chars.as_str().trim_start();
    if rest.is_empty() {
        return None;
    }

    let (head, path_and_ref) = rest.split_once(' ')?;
    let path_and_ref = path_and_ref.trim();
    if head.is_empty() || path_and_ref.is_empty() {
        return None;
    }

    let (path, ref_name) = if let Some((path, raw_ref)) = path_and_ref.rsplit_once(" (") {
        (
            path.trim(),
            raw_ref
                .strip_suffix(')')
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .map(str::to_string),
        )
    } else {
        (path_and_ref, None)
    };

    if path.is_empty() {
        return None;
    }

    Some(SubmoduleInfo {
        path: normalize_path_for_git(path),
        head: head.to_string(),
        ref_name,
        status: submodule_status_name(prefix).to_string(),
    })
}

#[tauri::command(async)]
fn get_submodules(state: State<'_, AppState>) -> Result<Vec<SubmoduleInfo>, String> {
    let repo = require_repo(&state)?;
    let raw = run_git_with_config(
        &repo,
        &[("core.quotepath", "false")],
        &["submodule", "status", "--recursive"],
    )?;
    Ok(raw
        .lines()
        .filter_map(parse_submodule_status_line)
        .collect())
}

#[tauri::command]
async fn update_submodule(state: State<'_, AppState>, path: String) -> Result<String, String> {
    let repo = require_repo(&state)?;
    let path = normalize_path_for_git(&path);
    if path.trim().is_empty() {
        return Err("submodule path 不能为空".into());
    }
    let result = tokio::task::spawn_blocking(move || {
        run_git(
            &repo,
            &["submodule", "update", "--init", "--recursive", "--", &path],
        )
    })
    .await
    .map_err(|e| format!("task failed: {e}"))?;
    result.map(|s| if s.is_empty() { "ok".into() } else { s })
}

// === Tags ===

#[tauri::command]
fn create_tag(
    state: State<'_, AppState>,
    name: String,
    message: Option<String>,
) -> Result<String, String> {
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

#[tauri::command(async)]
fn get_tags(state: State<'_, AppState>) -> Result<Vec<String>, String> {
    let repo = require_repo(&state)?;
    let raw = run_git(&repo, &["tag", "--sort=-creatordate"])?;
    Ok(raw
        .lines()
        .map(|l| l.trim().to_string())
        .filter(|l| !l.is_empty())
        .collect())
}

// === Stash ===

#[tauri::command]
fn stash_save(
    state: State<'_, AppState>,
    message: Option<String>,
    include_untracked: bool,
) -> Result<String, String> {
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

#[tauri::command(async)]
fn stash_list(state: State<'_, AppState>) -> Result<Vec<StashEntry>, String> {
    let repo = require_repo(&state)?;
    let raw = run_git(&repo, &["stash", "list"])?;
    let mut entries = Vec::new();
    for line in raw.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        // stash@{0}: On branch-name: message
        let index = entries.len();
        let rest = trimmed.split_once(": ").map(|(_, r)| r).unwrap_or(trimmed);
        let branch = rest
            .split_once(": ")
            .map(|(_, _r)| {
                // Try to extract branch name from "On branch-name: message"
                let branch_part = rest
                    .strip_prefix("On ")
                    .and_then(|s| s.split_once(": "))
                    .map(|(b, _)| b.to_string());
                let msg = rest
                    .split_once(": ")
                    .map(|(_, m)| m.to_string())
                    .unwrap_or_default();
                (branch_part.unwrap_or_default(), msg)
            })
            .unwrap_or((String::new(), rest.to_string()));
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

const AI_DIFF_BUDGET: usize = 60_000;
const AI_DIFF_RESERVE: usize = 5_000;
const AI_DIFF_TRUNCATE_LINES: usize = 150;
const AI_LOCK_HINT_LINES: usize = 500;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AiFileDiff {
    pub path: String,
    pub status: String,
    pub diff: String,
    pub truncated: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AiOmittedFile {
    pub path: String,
    pub status: String,
    pub additions: u32,
    pub deletions: u32,
    pub reason: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub hint: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AiStagedDiffContext {
    pub current_branch: String,
    pub summary: String,
    pub prompt_body: String,
    pub file_diffs: Vec<AiFileDiff>,
    pub omitted_files: Vec<AiOmittedFile>,
}

#[derive(Debug)]
struct StagedFileInfo {
    path: String,
    status: String,
    additions: u32,
    deletions: u32,
    is_binary: bool,
}

#[derive(Debug, PartialEq, Eq)]
enum FileTier {
    Source,
    Noise,
    Binary,
}

fn file_tier(path: &str, is_binary: bool) -> FileTier {
    if is_binary {
        return FileTier::Binary;
    }
    if is_noise_file(path) {
        return FileTier::Noise;
    }
    FileTier::Source
}

fn is_noise_file(path: &str) -> bool {
    let lower = path.to_lowercase();
    let name = path.rsplit('/').next().unwrap_or(path);
    matches!(
        name,
        "package-lock.json"
            | "yarn.lock"
            | "pnpm-lock.yaml"
            | "pnpm-lock.yml"
            | "Cargo.lock"
            | "poetry.lock"
            | "Gemfile.lock"
            | "composer.lock"
            | "go.sum"
            | "flake.lock"
            | "Podfile.lock"
            | "mix.lock"
    ) || lower.ends_with(".lock")
        || lower.contains("/node_modules/")
        || lower.ends_with(".min.js")
        || lower.ends_with(".min.css")
        || lower.ends_with(".map")
        || lower.ends_with(".snap")
        || lower.contains("/dist/")
        || lower.contains("/build/")
        || lower.starts_with("dist/")
        || lower.starts_with("build/")
}

fn parse_staged_files(repo: &str) -> Result<Vec<StagedFileInfo>, String> {
    let name_status = run_git(repo, &["diff", "--cached", "--name-status"])?;
    let numstat = run_git(repo, &["diff", "--cached", "--numstat"])?;

    let mut stats: std::collections::HashMap<String, (u32, u32, bool)> =
        std::collections::HashMap::new();
    for line in numstat.lines() {
        if line.is_empty() {
            continue;
        }
        let mut parts = line.split('\t');
        let add_s = parts.next().unwrap_or("0");
        let del_s = parts.next().unwrap_or("0");
        let path = parts.next().unwrap_or("").to_string();
        if path.is_empty() {
            continue;
        }
        let is_binary = add_s == "-" && del_s == "-";
        let additions = if is_binary {
            0
        } else {
            add_s.parse().unwrap_or(0)
        };
        let deletions = if is_binary {
            0
        } else {
            del_s.parse().unwrap_or(0)
        };
        stats.insert(path, (additions, deletions, is_binary));
    }

    let mut files = Vec::new();
    for line in name_status.lines() {
        if line.is_empty() {
            continue;
        }
        let mut parts = line.split('\t');
        let status_code = parts.next().unwrap_or("").to_string();
        if status_code.is_empty() {
            continue;
        }
        let status = match status_code.chars().next() {
            Some('A') => "Added",
            Some('D') => "Deleted",
            Some('M') => "Modified",
            Some('R') => "Renamed",
            Some('C') => "Copied",
            Some('T') => "Type changed",
            _ => "Changed",
        }
        .to_string();

        let path = if status_code.starts_with('R') || status_code.starts_with('C') {
            parts.nth(1).unwrap_or("").to_string()
        } else {
            parts.next().unwrap_or("").to_string()
        };
        if path.is_empty() {
            continue;
        }
        let (additions, deletions, is_binary) = stats.get(&path).copied().unwrap_or((0, 0, false));
        files.push(StagedFileInfo {
            path,
            status,
            additions,
            deletions,
            is_binary,
        });
    }
    Ok(files)
}

fn truncate_diff(diff: &str, max_lines: usize) -> (String, bool) {
    let lines: Vec<&str> = diff.lines().collect();
    if lines.len() <= max_lines {
        return (diff.to_string(), false);
    }
    let omitted = lines.len() - max_lines;
    let mut out = lines[..max_lines].join("\n");
    out.push_str(&format!("\n... [{omitted} lines omitted] ..."));
    (out, true)
}

fn extract_lock_hint(repo: &str, path: &str) -> Option<String> {
    let diff = run_git_diff(repo, &["diff", "--cached", "-U0", "--", path]).ok()?;
    let mut seen = std::collections::HashSet::new();
    let mut packages: Vec<String> = Vec::new();
    for line in diff.lines().take(AI_LOCK_HINT_LINES) {
        if !(line.starts_with('+') || line.starts_with('-')) {
            continue;
        }
        if line.starts_with("+++") || line.starts_with("---") {
            continue;
        }
        let content = &line[1..];
        for segment in content.split(|c: char| {
            !c.is_alphanumeric() && c != '@' && c != '/' && c != '-' && c != '_' && c != '.'
        }) {
            let s = segment.trim();
            if s.len() < 2 || s.len() > 80 {
                continue;
            }
            let looks_like_pkg = s.contains('@')
                || (s.contains('-') && !s.starts_with("node_modules"))
                || s.starts_with("node_modules/");
            if !looks_like_pkg {
                continue;
            }
            let name = s
                .strip_prefix("node_modules/")
                .unwrap_or(s)
                .split('/')
                .next()
                .unwrap_or(s)
                .to_string();
            if name.len() >= 2 && seen.insert(name.clone()) {
                packages.push(name);
                if packages.len() >= 12 {
                    break;
                }
            }
        }
        if packages.len() >= 12 {
            break;
        }
    }
    if packages.is_empty() {
        None
    } else {
        Some(format!(
            "changed packages (sample): {}",
            packages.join(", ")
        ))
    }
}

fn current_branch_name(repo: &str) -> String {
    run_git(repo, &["rev-parse", "--abbrev-ref", "HEAD"])
        .map(|s| s.trim().to_string())
        .unwrap_or_else(|_| "unknown".to_string())
}

fn build_ai_staged_diff_context(repo: &str) -> Result<AiStagedDiffContext, String> {
    let current_branch = current_branch_name(repo);
    let files = parse_staged_files(repo)?;
    if files.is_empty() {
        return Ok(AiStagedDiffContext {
            current_branch: current_branch.clone(),
            summary: "No staged changes.".to_string(),
            prompt_body: format!(
                "## Current Branch\n{current_branch}\n\n## Change Summary\nNo staged changes."
            ),
            file_diffs: Vec::new(),
            omitted_files: Vec::new(),
        });
    }

    let total_add: u32 = files.iter().map(|f| f.additions).sum();
    let total_del: u32 = files.iter().map(|f| f.deletions).sum();

    let mut summary = format!(
        "{} file(s) changed, +{} / -{} lines\n",
        files.len(),
        total_add,
        total_del
    );
    for f in &files {
        if f.is_binary {
            summary.push_str(&format!("- {} {} (binary)\n", f.status, f.path));
        } else {
            summary.push_str(&format!(
                "- {} {} (+{} / -{})\n",
                f.status, f.path, f.additions, f.deletions
            ));
        }
    }

    let mut omitted_files: Vec<AiOmittedFile> = Vec::new();
    let mut file_diffs: Vec<AiFileDiff> = Vec::new();
    let mut budget = AI_DIFF_BUDGET.saturating_sub(AI_DIFF_RESERVE);

    let mut source_indices: Vec<usize> = files
        .iter()
        .enumerate()
        .filter(|(_, f)| file_tier(&f.path, f.is_binary) == FileTier::Source)
        .map(|(i, _)| i)
        .collect();
    source_indices.sort_by_key(|&i| files[i].additions + files[i].deletions);

    for &idx in &source_indices {
        let f = &files[idx];
        let diff = run_git_diff(repo, &["diff", "--cached", "-U3", "--", &f.path])?;
        if diff.trim().is_empty() {
            continue;
        }

        if diff.len() <= budget {
            budget = budget.saturating_sub(diff.len());
            file_diffs.push(AiFileDiff {
                path: f.path.clone(),
                status: f.status.clone(),
                diff,
                truncated: false,
            });
            continue;
        }

        if budget < 400 {
            omitted_files.push(AiOmittedFile {
                path: f.path.clone(),
                status: f.status.clone(),
                additions: f.additions,
                deletions: f.deletions,
                reason: "budget-exhausted".to_string(),
                hint: None,
            });
            continue;
        }

        let max_lines = (budget / 60).clamp(30, AI_DIFF_TRUNCATE_LINES);
        let (truncated_diff, truncated) = truncate_diff(&diff, max_lines);
        if truncated_diff.len() > budget {
            omitted_files.push(AiOmittedFile {
                path: f.path.clone(),
                status: f.status.clone(),
                additions: f.additions,
                deletions: f.deletions,
                reason: "too-large".to_string(),
                hint: None,
            });
            continue;
        }
        budget = budget.saturating_sub(truncated_diff.len());
        file_diffs.push(AiFileDiff {
            path: f.path.clone(),
            status: f.status.clone(),
            diff: truncated_diff,
            truncated,
        });
    }

    for f in &files {
        let tier = file_tier(&f.path, f.is_binary);
        if tier == FileTier::Source {
            let included = file_diffs.iter().any(|d| d.path == f.path);
            let omitted = omitted_files.iter().any(|o| o.path == f.path);
            if !included && !omitted {
                omitted_files.push(AiOmittedFile {
                    path: f.path.clone(),
                    status: f.status.clone(),
                    additions: f.additions,
                    deletions: f.deletions,
                    reason: "empty-diff".to_string(),
                    hint: None,
                });
            }
            continue;
        }

        let reason = match tier {
            FileTier::Binary => "binary",
            FileTier::Noise => "generated-or-lock",
            FileTier::Source => unreachable!(),
        }
        .to_string();

        let hint = if tier == FileTier::Noise && f.additions + f.deletions <= 2000 {
            extract_lock_hint(repo, &f.path)
        } else if tier == FileTier::Noise {
            Some("large generated/lock file (full diff omitted)".to_string())
        } else {
            None
        };

        omitted_files.push(AiOmittedFile {
            path: f.path.clone(),
            status: f.status.clone(),
            additions: f.additions,
            deletions: f.deletions,
            reason,
            hint,
        });
    }

    let mut prompt_body =
        format!("## Current Branch\n{current_branch}\n\n## Change Summary\n{summary}");
    if !omitted_files.is_empty() {
        prompt_body.push_str("\n## Omitted Files (metadata only, no full diff)\n");
        for o in &omitted_files {
            if o.additions == 0 && o.deletions == 0 {
                prompt_body.push_str(&format!("- {} {} [{}]\n", o.status, o.path, o.reason));
            } else {
                prompt_body.push_str(&format!(
                    "- {} {} (+{} / -{}) [{}]",
                    o.status, o.path, o.additions, o.deletions, o.reason
                ));
            }
            if let Some(h) = &o.hint {
                prompt_body.push_str(&format!(" — {h}"));
            }
            prompt_body.push('\n');
        }
    }
    if !file_diffs.is_empty() {
        prompt_body.push_str("\n## Detailed Diffs (source files)\n");
        for fd in &file_diffs {
            prompt_body.push_str(&format!("\n### {} ({})\n", fd.path, fd.status));
            if fd.truncated {
                prompt_body.push_str("_Note: diff truncated due to size._\n");
            }
            prompt_body.push_str("```diff\n");
            prompt_body.push_str(&fd.diff);
            if !fd.diff.ends_with('\n') {
                prompt_body.push('\n');
            }
            prompt_body.push_str("```\n");
        }
    }

    Ok(AiStagedDiffContext {
        current_branch,
        summary: summary.trim_end().to_string(),
        prompt_body,
        file_diffs,
        omitted_files,
    })
}

#[tauri::command]
fn get_staged_diff(state: State<'_, AppState>) -> Result<String, String> {
    let repo = require_repo(&state)?;
    run_git(&repo, &["diff", "--cached"])
}

#[tauri::command]
fn get_staged_diff_for_ai(state: State<'_, AppState>) -> Result<AiStagedDiffContext, String> {
    let repo = require_repo(&state)?;
    build_ai_staged_diff_context(&repo)
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
                // ModelConfig {
                //     id: "model-openai-1".to_string(),
                //     provider_id: "openai-default".to_string(),
                //     name: "gpt-4o".to_string(),
                //     is_default: true,
                // },
                // ModelConfig {
                //     id: "model-anthropic-1".to_string(),
                //     provider_id: "anthropic-default".to_string(),
                //     name: "claude-sonnet-4-20250514".to_string(),
                //     is_default: false,
                // },
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
fn set_git_config(
    state: State<'_, AppState>,
    user_name: String,
    user_email: String,
) -> Result<(), String> {
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
    let content = fs::read_to_string(&path).map_err(|e| format!("failed to read settings: {e}"))?;
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
        .plugin(tauri_plugin_http::init())
        .manage(AppState {
            repo_path: Mutex::new(None),
            git_gate: Arc::new(Semaphore::new(1)),
        })
        .invoke_handler(tauri::generate_handler![
            open_repository,
            open_repository_at_path,
            get_repo_path,
            get_recent_repos,
            switch_repository,
            get_git_status,
            stage_file,
            unstage_file,
            revert_file,
            delete_file,
            commit_changes,
            amend_last_commit,
            soft_reset_last_commit,
            get_file_diff,
            get_git_log,
            get_branches,
            get_branch_tips,
            get_commit_diff,
            get_binary_image_preview,
            stage_patch,
            revert_patch,
            rename_branch,
            delete_branch,
            merge_branch,
            create_branch,
            create_branch_at,
            get_worktree_state,
            get_operation_state,
            continue_operation,
            abort_operation,
            mark_file_resolved,
            read_working_file,
            write_working_file,
            cherry_pick_commit,
            revert_commit,
            checkout_branch,
            checkout_remote_branch,
            checkout_with_mode,
            git_fetch,
            get_ahead_behind,
            git_push,
            git_pull,
            pin_branch,
            unpin_branch,
            get_pinned_branches,
            create_tag,
            get_tags,
            get_subtrees,
            subtree_pull,
            subtree_push,
            get_submodules,
            update_submodule,
            stash_save,
            stash_list,
            stash_apply,
            stash_drop,
            stash_file,
            get_staged_diff,
            get_staged_diff_for_ai,
            clone_repository,
            get_git_config,
            set_git_config,
            load_settings,
            save_settings,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_git_status_porcelain_v2_z_reads_changed_untracked_and_renamed() {
        let raw = b"# branch.head main\0\
1 .M N... 100644 100644 100644 abc abc src/main.rs\0\
1 M. N... 100644 100644 100644 abc def src/lib with space.rs\0\
2 R. N... 100644 100644 100644 abc def R100 src/new.rs\0src/old.rs\0\
? notes/new file.md\0";
        let items = parse_git_status_porcelain_v2_z(raw);

        assert_eq!(
            items
                .iter()
                .map(|item| (item.path.as_str(), item.status.as_str(), item.is_staged))
                .collect::<Vec<_>>(),
            vec![
                ("src/main.rs", " M", false),
                ("src/lib with space.rs", "M ", true),
                ("src/new.rs", "R ", true),
                ("notes/new file.md", "??", false),
            ]
        );
    }

    #[test]
    fn parse_submodule_status_line_reads_clean_submodule() {
        let item = parse_submodule_status_line(" 9fceb02 vendor/lib (heads/main)").unwrap();

        assert_eq!(item.path, "vendor/lib");
        assert_eq!(item.head, "9fceb02");
        assert_eq!(item.ref_name.as_deref(), Some("heads/main"));
        assert_eq!(item.status, "clean");
    }

    #[test]
    fn parse_submodule_status_line_keeps_paths_with_spaces() {
        let item = parse_submodule_status_line("+abc1234 third party/lib name (v1.0.0)").unwrap();

        assert_eq!(item.path, "third party/lib name");
        assert_eq!(item.head, "abc1234");
        assert_eq!(item.ref_name.as_deref(), Some("v1.0.0"));
        assert_eq!(item.status, "modified");
    }

    #[test]
    fn parse_submodule_status_line_marks_uninitialized() {
        let item = parse_submodule_status_line("-abc1234 modules/core").unwrap();

        assert_eq!(item.path, "modules/core");
        assert_eq!(item.ref_name, None);
        assert_eq!(item.status, "uninitialized");
    }
}
