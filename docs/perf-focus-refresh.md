# Windows 下窗口聚焦（onFocus）刷新缓慢 — 优化实施方案

> 本文档面向执行本任务的 AI 助手。请**严格按任务顺序逐个执行**，每完成一个任务必须运行对应的验证命令，验证通过后才能开始下一个任务。不要自行发挥、不要重命名现有函数/命令、不要引入新依赖。

---

## 1. 背景与症状

本项目是 Tauri 2 + Vue 3 的 Git 客户端（前端 `src/`，Rust 后端 `src-tauri/src/lib.rs`）。

症状：在公司 Windows 电脑上，每次窗口重新聚焦时，界面要"等一会儿"才恢复流畅。

原因：窗口聚焦会触发一次全量刷新，而全量刷新会**串行/并发地启动 10~15 个 `git.exe` 子进程**。Windows 上进程创建开销远高于 macOS/Linux，且部分 Tauri 命令是同步命令（运行在主线程上），每个 git 进程执行期间整个 UI 都被阻塞。

## 2. 现状代码梳理（改动前必读）

### 2.1 前端刷新链路（`src/App.vue`）

聚焦监听（`onMounted` 内，约 212-213 行）：

```ts
window.addEventListener('focus', refreshWorkspaceIfVisible)
document.addEventListener('visibilitychange', refreshWorkspaceIfVisible)
```

聚焦后的处理函数（约 167-179 行）：

```ts
function refreshWorkspaceIfVisible() {
  if (!repoPath.value) return
  if (document.visibilityState === 'hidden') return
  if (debouncedRefreshTimer) clearTimeout(debouncedRefreshTimer)
  debouncedRefreshTimer = setTimeout(() => {
    debouncedRefreshTimer = null
    void syncRefresh({ silentStatus: true })
  }, 400)
}
```

`syncRefresh`（约 317-330 行）——聚焦时会触发下面**全部**刷新：

```ts
async function syncRefresh(opts?: { silentStatus?: boolean }) {
  const tasks = [
    refreshStatus(opts?.silentStatus ? { silent: true } : undefined),
    refreshBranches(),
    refreshAheadBehind(),
    refreshSubtrees(),
    refreshSubmodules(),
    refreshOperationState(),
  ]
  if (activeTab.value === 'history') {
    tasks.push(refreshHistory())
  }
  await Promise.all(tasks)
}
```

`refreshBranches`（约 225-236 行）——两次 invoke 是**串行 await**：

```ts
async function refreshBranches() {
  if (!repoPath.value) return
  branchesLoading.value = true
  try {
    branches.value = await invoke<BranchInfo[]>('get_branches')
    branchTips.value = await invoke<BranchTip[]>('get_branch_tips')
  } catch (e: any) {
    showToast(String(e))
  } finally {
    branchesLoading.value = false
  }
}
```

### 2.2 后端命令（`src-tauri/src/lib.rs`）

聚焦链路涉及的后端命令（**均为同步命令**，运行在主线程，执行期间阻塞 UI）：

| 命令 | 位置（约） | 启动的 git 进程数 |
|---|---|---|
| `get_git_status` | 1003 行（已 async + spawn_blocking） | 1 |
| `get_branches` | 1259 行 | 1（`git branch -a`） |
| `get_branch_tips` | 1266 行 | 1（`git for-each-ref`） |
| `get_operation_state` | 1365 行 | 0~2（文件系统检查 + 冲突时 1 次 diff） |
| `get_ahead_behind` | 1791 行 | 约 4 次（rev-parse ×2、rev-list ×2） |
| `get_subtrees` | 1977 行 | ≥1 |
| `get_submodules` | 2094 行 | 1（**`git submodule status --recursive`，Windows 上非常慢**） |
| `get_tags` | 2151 行 | 1 |
| `stash_list` | 2184 行 | 1 |

已有的性能日志：`log_git_perf`（约 230 行）会把每条 git 命令的 `elapsed_ms` 打到 stderr，格式为 `[gitwave:git] success=... elapsed_ms=... command="git ..."`。可用它在 Windows 机器上量化效果。

### 2.3 根因小结

1. 每次聚焦触发全量刷新，包括低频数据（submodules / subtrees / ahead-behind）。
2. 热路径上的后端命令是同步命令，git 子进程运行期间阻塞主线程。
3. `refreshBranches` 内两次 invoke 串行。
4. Windows 上未开启 `core.fsmonitor` / `core.untrackedCache`，`git status` 每次全量扫描工作区。

## 3. 实施总则（执行前必读）

- **一次只做一个任务**，做完立即验证。
- 行号是编写本文档时的参考值，可能有偏移。**定位代码请以"搜索锚点"（代码片段）为准**，用文本搜索找到实际位置。
- 不修改任何 Tauri command 的**名称**和**参数/返回结构**（除非任务明确要求新增）。
- 不引入任何新的 crate / npm 依赖。
- 代码注释使用中文，风格与现有代码保持一致；不写多余注释。
- 每个任务完成后运行：`cargo fmt --manifest-path src-tauri/Cargo.toml`。
- 全部完成后运行第 6 节的完整验证清单。

---

## 阶段一：低风险高收益（必做）

## 任务 T1：`git status` 加 `--no-optional-locks`，并自动开启 fsmonitor / untrackedCache

### T1.1 `--no-optional-locks`

**文件**：`src-tauri/src/lib.rs`

**搜索锚点**（位于 `get_git_status` 命令内的 `spawn_blocking` 闭包中）：

```rust
&["status", "--porcelain=v2", "-z", "--branch"],
```

**改为**：

```rust
&["--no-optional-locks", "status", "--porcelain=v2", "-z", "--branch"],
```

说明：`run_git_with_config` 会把 args 拼在 `-c key=value` 之后、子命令之前，因此最终命令是 `git -c core.quotepath=false --no-optional-locks status ...`，`--no-optional-locks` 是 git 的全局选项，放在这个位置是合法的。它的作用是让 status 不获取 index 写锁，避免与后台进程互相阻塞。

同样的改法应用到 `has_worktree_changes` 函数中的：

```rust
&["status", "--porcelain"],
```

改为：

```rust
&["--no-optional-locks", "status", "--porcelain"],
```

**注意**：只改这两处。`revert_file` / `delete_file` 里的 `git status --porcelain -- <path>` 不改（它们是单文件操作，且改了语义无收益）。

### T1.2 自动开启加速配置

**文件**：`src-tauri/src/lib.rs`

**第 1 步**：在 `run_git` 函数定义（搜索锚点 `fn run_git(repo: &str, args: &[&str]) -> Result<String, String> {`）之前，新增一个辅助函数：

```rust
/// 尽力开启能显著加速 Windows 上 `git status` 的仓库本地配置。失败静默忽略。
fn ensure_fast_status_config(repo: &str) {
    let _ = run_git(repo, &["config", "core.fsmonitor", "true"]);
    let _ = run_git(repo, &["config", "core.untrackedCache", "true"]);
}
```

**第 2 步**：在 `open_repository_at` 函数（搜索锚点 `fn open_repository_at(`）的函数体末尾、`Ok(path_str)` 之前，插入一行：

```rust
    ensure_fast_status_config(&path_str);
```

**第 3 步**：在 `switch_repository` 函数（搜索锚点 `fn switch_repository(app: tauri::AppHandle, path: String)`）中，`save_last_repo(&app, &path)?;` 之后、`Ok(path)` 之前，插入：

```rust
    ensure_fast_status_config(&path);
```

说明：只写入仓库本地 config（`.git/config`），不动用户全局配置。旧版本 git 不支持 fsmonitor daemon 时该配置无副作用。

### T1 验证

```bash
cd src-tauri && cargo test && cargo fmt
```

`cargo test` 必须全部通过（已有测试位于 lib.rs 底部 `#[cfg(test)]` 模块）。可以仿照现有测试风格为 `ensure_fast_status_config` 补一个"调用不 panic"的测试，但不强制。

---

## 任务 T2：热路径只读命令改为非阻塞执行

Tauri 2 中，非 async 的 `#[tauri::command]` 在主线程执行，git 子进程运行期间会卡 UI。把聚焦刷新链路上的只读命令标记为 `(async)`（表示放到单独线程执行），函数体**完全不用改**。

**文件**：`src-tauri/src/lib.rs`

对下列 8 个命令，把 `#[tauri::command]` 改为 `#[tauri::command(async)]`（通过各自的 `fn 名字` 搜索定位，只改紧邻其上的那一行属性）：

1. `fn get_branches(`
2. `fn get_branch_tips(`
3. `fn get_operation_state(`
4. `fn get_ahead_behind(`
5. `fn get_subtrees(`
6. `fn get_submodules(`
7. `fn get_tags(`
8. `fn stash_list(`

**注意**：
- 全文还有很多其他 `#[tauri::command]`，**一个都不要动**，只改上面列出的 8 个。
- 如果编译报错（理论上不应发生），回退本任务并报告错误信息，改用备选方案：仿照 `get_git_status`（约 1003 行）的写法，把命令改成 `async fn` + `tokio::task::spawn_blocking` 包裹函数体，并使用 `state.git_gate` 信号量。

### T2 验证

```bash
cd src-tauri && cargo build && cargo fmt
```

---

## 任务 T3：前端聚焦刷新减负 + 节流

**文件**：`src/App.vue`

### T3.1 新增节流状态与聚焦刷新函数

在 `let historyRequestSeq = 0`（搜索锚点）之后新增：

```ts
/** 慢速刷新（ahead/behind、subtree、submodule）在聚焦时的最小间隔 */
const FOCUS_SLOW_REFRESH_INTERVAL_MS = 60_000
let lastSlowRefreshAt = 0
```

把 `refreshWorkspaceIfVisible` 整个函数（搜索锚点 `function refreshWorkspaceIfVisible()`）替换为：

```ts
function refreshWorkspaceIfVisible() {
  if (!repoPath.value) return
  if (document.visibilityState === 'hidden') return
  if (debouncedRefreshTimer) clearTimeout(debouncedRefreshTimer)
  debouncedRefreshTimer = setTimeout(() => {
    debouncedRefreshTimer = null
    void refreshOnFocus()
  }, 400)
}

/** 聚焦时只做轻量刷新；慢速数据按间隔节流，避免 Windows 上频繁拉起 git 进程 */
async function refreshOnFocus() {
  await Promise.all([
    refreshStatus({ silent: true }),
    refreshBranches(),
    refreshOperationState(),
  ])
  const now = Date.now()
  if (now - lastSlowRefreshAt < FOCUS_SLOW_REFRESH_INTERVAL_MS) return
  lastSlowRefreshAt = now
  await Promise.all([refreshAheadBehind(), refreshSubtrees(), refreshSubmodules()])
}
```

### T3.2 显式刷新重置节流计时

在 `syncRefresh` 函数体开头（搜索锚点 `async function syncRefresh(`，其 `const tasks = [` 之前）加一行：

```ts
  lastSlowRefreshAt = Date.now()
```

### T3.3 `refreshBranches` 内两次 invoke 并行

把 `refreshBranches` 中的：

```ts
    branches.value = await invoke<BranchInfo[]>('get_branches')
    branchTips.value = await invoke<BranchTip[]>('get_branch_tips')
```

替换为：

```ts
    const [nextBranches, nextTips] = await Promise.all([
      invoke<BranchInfo[]>('get_branches'),
      invoke<BranchTip[]>('get_branch_tips'),
    ])
    branches.value = nextBranches
    branchTips.value = nextTips
```

### T3 验证

```bash
pnpm build && pnpm test:unit
```

两者必须零报错（`pnpm build` 会先跑 `vue-tsc --noEmit` 类型检查）。

---

## 阶段二：中等收益（建议做）

## 任务 T4：`get_ahead_behind` 减少进程数

**文件**：`src-tauri/src/lib.rs`

现状：`get_ahead_behind` → `ahead_behind_for_branch` 固定执行 4 次 git 进程：`rev-parse --abbrev-ref HEAD`、`compare_ref_for_branch` 里的 upstream 解析、`parse_ahead_behind`（rev-list count）、`list_unpushed_hashes`（rev-list）。其中 ahead 为 0 时 unpushed_hashes 必为空，最后一次调用可以跳过。

**搜索锚点**：

```rust
fn ahead_behind_for_branch(repo: &str, branch: &str) -> Result<AheadBehind, String> {
    let compare = compare_ref_for_branch(repo, branch)?;
    let (ahead, behind) = parse_ahead_behind(repo, &format!("{compare}...HEAD"))?;
    let unpushed_hashes = list_unpushed_hashes(repo, &compare)?;
```

**改为**（只加一个条件判断）：

```rust
fn ahead_behind_for_branch(repo: &str, branch: &str) -> Result<AheadBehind, String> {
    let compare = compare_ref_for_branch(repo, branch)?;
    let (ahead, behind) = parse_ahead_behind(repo, &format!("{compare}...HEAD"))?;
    let unpushed_hashes = if ahead == 0 {
        Vec::new()
    } else {
        list_unpushed_hashes(repo, &compare)?
    };
```

（函数其余部分保持不变。）

### T4 验证

```bash
cd src-tauri && cargo test && cargo fmt
```

---

## 阶段三：可选（阶段一、二验证通过后再做，做不动可以放弃）

## 任务 T5（可选）：合并 `get_branches` + `get_branch_tips` 为单次 git 调用

**文件**：`src-tauri/src/lib.rs`、`src/App.vue`

背景：`get_branches` 用 `git branch -a`，`get_branch_tips` 用 `git for-each-ref`，两次进程可合并为一次 `for-each-ref`（带 `%(HEAD)` 标记识别当前分支）。

实施要点：

1. 新增命令 `get_branch_overview`，返回结构：

```rust
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BranchOverview {
    pub branches: Vec<BranchInfo>,
    pub tips: Vec<BranchTip>,
}
```

2. 命令内部用**一次** `git for-each-ref` 实现：

```text
git for-each-ref --format=%(refname:short)%00%(objectname)%00%(HEAD) refs/heads refs/remotes
```

3. `BranchInfo` 的字段必须与现有 `parse_branches`（搜索锚点 `fn parse_branches(`）的输出语义完全一致（`isCurrent` 来自 `%(HEAD)` 的 `*` 标记；remote 分支的 `isCurrent` 恒为 false）。先读懂 `parse_branches` 和 `BranchInfo` 结构再动手。
4. 前端 `refreshBranches` 改为调用新命令，一次 invoke 同时填充 `branches` 与 `branchTips`。
5. **保留**旧的 `get_branches` / `get_branch_tips` 两个命令不删（其他地方可能还在用）。

### T5 验证

```bash
cd src-tauri && cargo test && cargo fmt
pnpm build && pnpm test:unit
```

并手动验证（`pnpm tauri dev`）：当前分支高亮、分支列表、历史图分支 tip 标记均正常。

---

## 6. 最终验证清单（所有任务完成后）

```bash
cd src-tauri && cargo test && cargo fmt && cargo build
pnpm build
pnpm test:unit
pnpm tauri dev
```

手动回归项（`pnpm tauri dev` 启动后）：

1. 打开一个仓库 → 切到别的应用再切回来 → 界面应立即恢复交互，不卡顿。
2. 侧边栏 ahead/behind 徽标正常（有未推送提交时显示 ahead 数字）。
3. submodule / subtree 面板数据正常（注意：聚焦后最多 60 秒内不会刷新它们，这是预期行为；pull/push/切换仓库后会立即刷新）。
4. stash 列表、tag 列表正常。
5. 工作区 stage/unstage/commit 后状态刷新正常。

量化方式（Windows 机器上）：`pnpm tauri dev` 运行时观察终端 stderr 中 `[gitwave:git] success=... elapsed_ms=... command="git status ..."` 等日志，对比优化前后每次聚焦的总耗时与命令数量。

## 7. 风险与回滚

- 每个任务相互独立，出问题可单独回滚对应改动。
- T1.2 会往仓库本地 `.git/config` 写入 `core.fsmonitor=true` 和 `core.untrackedCache=true`；如需回滚，手动执行 `git config --unset core.fsmonitor` 和 `git config --unset core.untrackedCache`。这两项只影响性能，不影响 git 行为正确性。
- T3 改变了聚焦时的刷新范围：submodule/subtree/ahead-behind 最多每 60 秒随聚焦刷新一次。若产品上要求聚焦实时性，可调小 `FOCUS_SLOW_REFRESH_INTERVAL_MS`。
- T2 若出现编译问题，按任务内的备选方案处理或整体回滚。
