<script setup lang="ts">
import { ref, computed, onMounted, onUnmounted } from 'vue'
import { invoke } from '@tauri-apps/api/core'
import { getCurrentWebview } from '@tauri-apps/api/webview'
import { Splitpanes, Pane } from 'splitpanes'
import 'splitpanes/dist/splitpanes.css'
import SidebarPanel from './components/SidebarPanel.vue'
import WorkspacePanel from './components/WorkspacePanel.vue'
import DiffPanel from './components/DiffPanel.vue'
import HistoryTab from './components/HistoryTab.vue'
import SettingsDialog from './components/SettingsDialog.vue'
import { confirm } from '@tauri-apps/plugin-dialog'
import { Loader2 } from 'lucide-vue-next'
import type {
  FileStatus,
  CommitLog,
  CommitLogPage,
  BranchInfo,
  BranchTip,
  AheadBehind,
  WorktreeState,
  CheckoutMode,
  SubtreeInfo,
  SubmoduleInfo,
  OperationState,
  CommitAction,
  CommitPayload,
} from './types'
import { isUntrackedPath } from './utils/gitStatus'
import { applyOptimisticRevertToDiffText } from './utils/diffPatch'
import { applyFileListSelectionClick } from './utils/fileListSelection'

// State
const repoPath = ref<string | null>(null)
const activeTab = ref<'workspace' | 'history'>('workspace')
const statuses = ref<FileStatus[]>([])
const selectedFile = ref<string | null>(null)
/** 工作区列表多选（同区域内 Cmd/Ctrl、Shift 选择） */
const selectedFilePaths = ref<Set<string>>(new Set())
const selectionAnchor = ref<string | null>(null)
/** 工作区列表里当前选中项是否在 Staged 区域（用于二进制图片 diff 预览取哪一侧） */
const selectedFileIsStaged = ref(false)
const selectedCommitHash = ref<string | null>(null)
const selectedCommitMsg = ref('')
const diffText = ref('')
const commitLogs = ref<CommitLog[]>([])
const diffFileName = computed(() => {
  if (selectedCommitHash.value) return `commit ${selectedCommitHash.value} - ${selectedCommitMsg.value}`
  const count = selectedFilePaths.value.size
  if (count > 1) return `${count} 个文件`
  return selectedFile.value
})

const canStage = computed(() => !selectedCommitHash.value && !!selectedFile.value)

const branches = ref<BranchInfo[]>([])
const branchTips = ref<BranchTip[]>([])
const branchesLoading = ref(false)
const recentRepos = ref<string[]>([])
const pinnedBranches = ref<string[]>([])
const stashEntries = ref<any[]>([])
const tags = ref<string[]>([])
const subtrees = ref<SubtreeInfo[]>([])
const submodules = ref<SubmoduleInfo[]>([])
const subtreeActionLoading = ref<string | null>(null)
const submoduleActionLoading = ref<string | null>(null)
const settingsOpen = ref(false)
const settingsRevision = ref(0)
/** 提交成功后递增，驱动工作区「已提交」短提示（避免用 commitLoading 启发式推断） */
const commitSuccessTick = ref(0)

const historyFilter = ref<'current' | 'all'>('current')
const currentBranch = computed(() => branches.value.find(b => b.isCurrent)?.name ?? '')

const statusLoading = ref(false)
const commitLoading = ref(false)
const historyLoading = ref(false)
const historyLoadingMore = ref(false)
const historyHasMore = ref(false)
const HISTORY_PAGE_SIZE = 50
const pushLoading = ref(false)
const pullLoading = ref(false)
const aheadBehind = ref<AheadBehind>({ ahead: 0, behind: 0, unpushedHashes: [] })
const fetchLoading = ref(false)
const patchStaging = ref(false)
const operationState = ref<OperationState>({
  kind: 'none',
  conflictedFiles: [],
  hasConflicts: false,
  canContinue: false,
  canAbort: false,
})

const operationActive = computed(() => operationState.value.kind !== 'none')
const operationLabel = computed(() => {
  const state = operationState.value
  const name =
    state.kind === 'merge'
      ? '合并'
      : state.kind === 'rebase'
        ? 'Rebase'
        : state.kind === 'cherryPick'
          ? 'Cherry-pick'
          : ''
  if (!name) return ''
  if (state.hasConflicts) return `${name} 冲突中 · ${state.conflictedFiles.length} 个文件`
  return `${name} 进行中 · 等待继续`
})

// Global refresh indicator (syncs across panels)
const globalRefreshing = computed(() => branchesLoading.value || historyLoading.value || statusLoading.value)

// Toast
const toast = ref<{ message: string; type: 'error' | 'success' } | null>(null)
let toastTimer: ReturnType<typeof setTimeout> | null = null

function clearFileSelection() {
  diffRequestSeq++
  selectedFile.value = null
  selectedFileIsStaged.value = false
  selectedFilePaths.value = new Set()
  selectionAnchor.value = null
  diffText.value = ''
}

function showToast(message: string, type: 'error' | 'success' = 'error') {
  toast.value = { message, type }
  if (toastTimer) clearTimeout(toastTimer)
  toastTimer = setTimeout(() => {
    toast.value = null
  }, 6000)
}

const repoDragOver = ref(false)
let unlistenDragDrop: (() => void) | null = null

async function applyOpenedRepo(path: string) {
  repoPath.value = path
  showToast('仓库已打开', 'success')
  await Promise.all([syncRefresh(), refreshRecentRepos(), refreshTags(), stashList(), refreshSubmodules()])
}

// Open repository
async function openRepo() {
  try {
    const path = await invoke<string>('open_repository')
    await applyOpenedRepo(path)
  } catch (e: any) {
    if (e !== 'dialog cancelled') {
      showToast(String(e))
    }
  }
}

async function openRepoFromDrop(paths: string[]) {
  const path = paths.find((p) => p.trim())
  if (!path) return
  try {
    const opened = await invoke<string>('open_repository_at_path', { path: path.trim() })
    await applyOpenedRepo(opened)
  } catch (e: any) {
    showToast(String(e))
  }
}

let debouncedRefreshTimer: ReturnType<typeof setTimeout> | null = null
let statusRequestSeq = 0
let historyRequestSeq = 0
let diffRequestSeq = 0
/** 慢速刷新（ahead/behind、subtree、submodule）在聚焦时的最小间隔 */
const FOCUS_SLOW_REFRESH_INTERVAL_MS = 60_000
let lastSlowRefreshAt = 0

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

// Get repo path on mount
onMounted(async () => {
  try {
    const path = await invoke<string | null>('get_repo_path')
    repoPath.value = path
    if (path) {
    await Promise.all([refreshStatus(), refreshBranches(), refreshAheadBehind(), stashList(), refreshSubtrees(), refreshSubmodules(), refreshOperationState()])
    }
  } catch (_) {
    // ignore
  }
  // Always load recent repos regardless of current session state
  await refreshRecentRepos()
  await refreshPinnedBranches()
  await refreshTags()

  try {
    unlistenDragDrop = await getCurrentWebview().onDragDropEvent((event) => {
      if (event.payload.type === 'over' || event.payload.type === 'enter') {
        repoDragOver.value = true
      } else if (event.payload.type === 'drop') {
        repoDragOver.value = false
        void openRepoFromDrop(event.payload.paths)
      } else {
        repoDragOver.value = false
      }
    })
  } catch (_) {
    // ignore when not running inside Tauri
  }

  window.addEventListener('focus', refreshWorkspaceIfVisible)
  document.addEventListener('visibilitychange', refreshWorkspaceIfVisible)
})

onUnmounted(() => {
  if (debouncedRefreshTimer) clearTimeout(debouncedRefreshTimer)
  unlistenDragDrop?.()
  unlistenDragDrop = null
  window.removeEventListener('focus', refreshWorkspaceIfVisible)
  document.removeEventListener('visibilitychange', refreshWorkspaceIfVisible)
})

// Refresh branches
async function refreshBranches() {
  if (!repoPath.value) return
  branchesLoading.value = true
  try {
    const [nextBranches, nextTips] = await Promise.all([
      invoke<BranchInfo[]>('get_branches'),
      invoke<BranchTip[]>('get_branch_tips'),
    ])
    branches.value = nextBranches
    branchTips.value = nextTips
  } catch (e: any) {
    showToast(String(e))
  } finally {
    branchesLoading.value = false
  }
}

// Refresh recent repos
async function refreshRecentRepos() {
  try {
    recentRepos.value = await invoke<string[]>('get_recent_repos')
  } catch (_) {
    // ignore
  }
}

async function removeRecentRepo(path: string) {
  try {
    recentRepos.value = await invoke<string[]>('remove_recent_repo', { path })
  } catch (e: any) {
    showToast(String(e))
  }
}

async function cloneRepo(url: string, targetDir: string) {
  try {
    const path = await invoke<string>('clone_repository', { url, targetDir })
    repoPath.value = path
    showToast('仓库克隆成功', 'success')
    await Promise.all([syncRefresh(), refreshRecentRepos(), refreshTags(), refreshSubtrees(), refreshSubmodules()])
  } catch (e: any) {
    showToast(String(e))
  }
}

async function refreshPinnedBranches() {
  try {
    pinnedBranches.value = await invoke<string[]>('get_pinned_branches')
  } catch (_) {
    // ignore
  }
}

async function refreshTags() {
  if (!repoPath.value) return
  try {
    tags.value = await invoke<string[]>('get_tags')
  } catch (_) {
    // ignore
  }
}

async function refreshSubtrees() {
  if (!repoPath.value) return
  try {
    subtrees.value = await invoke<SubtreeInfo[]>('get_subtrees')
  } catch (_) {
    subtrees.value = []
  }
}

async function refreshSubmodules() {
  if (!repoPath.value) return
  try {
    submodules.value = await invoke<SubmoduleInfo[]>('get_submodules')
  } catch (_) {
    submodules.value = []
  }
}

async function refreshOperationState() {
  if (!repoPath.value) {
    operationState.value = {
      kind: 'none',
      conflictedFiles: [],
      hasConflicts: false,
      canContinue: false,
      canAbort: false,
    }
    return
  }
  try {
    operationState.value = await invoke<OperationState>('get_operation_state')
  } catch (_) {
    operationState.value = {
      kind: 'none',
      conflictedFiles: [],
      hasConflicts: false,
      canContinue: false,
      canAbort: false,
    }
  }
}

async function syncRefresh(opts?: { silentStatus?: boolean }) {
  lastSlowRefreshAt = Date.now()
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

// Switch to a known repo
async function switchRepo(path: string) {
  try {
    await invoke<string>('switch_repository', { path })
    repoPath.value = path
    showToast('已切换仓库', 'success')
    await syncRefresh()
    await refreshTags()
    await refreshSubtrees()
    await refreshSubmodules()
  } catch (e: any) {
    showToast(String(e))
  }
}

// Refresh status（silent：后台同步，不挡整个列表的加载态）
async function refreshStatus(opts?: { silent?: boolean }) {
  if (!repoPath.value) return
  const requestId = ++statusRequestSeq
  const silent = opts?.silent ?? false
  if (!silent) statusLoading.value = true
  try {
    const nextStatuses = await invoke<FileStatus[]>('get_git_status')
    if (requestId !== statusRequestSeq) return
    statuses.value = nextStatuses
    const paths = statuses.value.map((s) => s.path)
    selectedFilePaths.value = new Set([...selectedFilePaths.value].filter((p) => paths.includes(p)))
    if (selectedFile.value && !paths.includes(selectedFile.value)) {
      clearFileSelection()
    }
  } catch (e: any) {
    if (!silent) showToast(String(e))
  } finally {
    if (!silent) statusLoading.value = false
  }
}

function normalizePaths(pathOrPaths: string | string[]): string[] {
  return (Array.isArray(pathOrPaths) ? pathOrPaths : [pathOrPaths]).filter(Boolean)
}

function removePathsFromSelection(paths: string[]) {
  const next = new Set(selectedFilePaths.value)
  for (const path of paths) next.delete(path)
  selectedFilePaths.value = next
}

function orderedSelectedWorkspacePaths(): string[] {
  const selected = selectedFilePaths.value
  const ordered: string[] = []
  const seen = new Set<string>()
  for (const status of statuses.value) {
    if (status.isStaged !== selectedFileIsStaged.value) continue
    if (!selected.has(status.path) || seen.has(status.path)) continue
    seen.add(status.path)
    ordered.push(status.path)
  }
  return ordered
}

function reconcileWorkspaceSelection(): string[] {
  const ordered = orderedSelectedWorkspacePaths()
  selectedFilePaths.value = new Set(ordered)
  if (ordered.length === 0) {
    selectedFile.value = null
    selectionAnchor.value = null
    diffText.value = ''
    return []
  }
  if (!selectedFile.value || !ordered.includes(selectedFile.value)) {
    selectedFile.value = ordered[0]
  }
  return ordered
}

async function loadWorkspaceDiffs(paths: string[], isStaged: boolean) {
  const req = ++diffRequestSeq
  if (paths.length === 0) {
    if (req === diffRequestSeq) diffText.value = ''
    return
  }
  try {
    const text = await invoke<string>('get_files_diff', { paths, isStaged })
    if (req !== diffRequestSeq) return
    diffText.value = text
  } catch (e: any) {
    if (req !== diffRequestSeq) return
    diffText.value = ''
    showToast(String(e))
  }
}

async function syncSelectedWorkspaceDiff() {
  const paths = reconcileWorkspaceSelection()
  if (paths.length === 0) return
  await loadWorkspaceDiffs(paths, selectedFileIsStaged.value)
}

// Stage / Unstage
async function stageFile(pathOrPaths: string | string[]) {
  const paths = normalizePaths(pathOrPaths)
  if (paths.length === 0) return
  try {
    for (const path of paths) {
      await invoke('stage_file', { path })
    }
    await refreshStatus()
    removePathsFromSelection(paths)
    await syncSelectedWorkspaceDiff()
  } catch (e: any) {
    showToast(String(e))
  }
}

async function unstageFile(pathOrPaths: string | string[]) {
  const paths = normalizePaths(pathOrPaths)
  if (paths.length === 0) return
  try {
    for (const path of paths) {
      await invoke('unstage_file', { path })
    }
    await refreshStatus()
    removePathsFromSelection(paths)
    await syncSelectedWorkspaceDiff()
  } catch (e: any) {
    showToast(String(e))
  }
}

function revertConfirmMessage(paths: string[], isStaged: boolean): string {
  if (paths.length === 1) {
    const path = paths[0]
    const untracked = !isStaged && isUntrackedPath(path, statuses.value)
    if (untracked) return `确认删除未跟踪文件「${path}」？此操作不可恢复。`
    if (isStaged) return `确认丢弃「${path}」的全部变更（含已 Stage）？此操作不可撤销。`
    return `确认丢弃「${path}」的工作区变更？此操作不可撤销。`
  }
  return isStaged
    ? `确认丢弃 ${paths.length} 个文件的全部变更（含已 Stage）？此操作不可撤销。`
    : `确认丢弃 ${paths.length} 个文件的工作区变更？此操作不可撤销。`
}

async function revertFile(pathOrPaths: string | string[], isStaged: boolean) {
  const paths = normalizePaths(pathOrPaths)
  if (paths.length === 0) return
  if (!(await confirm(revertConfirmMessage(paths, isStaged)))) return
  try {
    for (const path of paths) {
      await invoke('revert_file', { path, isStaged })
    }
    await refreshStatus()
    removePathsFromSelection(paths)
    await syncSelectedWorkspaceDiff()
    showToast('已丢弃变更', 'success')
  } catch (e: any) {
    showToast(String(e))
  }
}

async function deleteFile(pathOrPaths: string | string[], isStaged: boolean) {
  const paths = normalizePaths(pathOrPaths)
  if (paths.length === 0) return
  const msg =
    paths.length === 1
      ? `确认删除文件「${paths[0]}」？此操作不可恢复。`
      : `确认删除 ${paths.length} 个文件？此操作不可恢复。`
  if (!(await confirm(msg))) return
  try {
    for (const path of paths) {
      await invoke('delete_file', { path, isStaged })
    }
    await refreshStatus()
    removePathsFromSelection(paths)
    await syncSelectedWorkspaceDiff()
    showToast('已删除文件', 'success')
  } catch (e: any) {
    showToast(String(e))
  }
}

// Commit
async function commitChanges(payload: CommitPayload) {
  commitLoading.value = true
  let ok = false
  try {
    const command = payload.amend ? 'amend_last_commit' : 'commit_changes'
    await invoke(command, { message: payload.message })
    showToast(payload.amend ? '已修正最后一次提交' : '提交成功', 'success')
    await Promise.all([refreshStatus(), refreshAheadBehind(), refreshSubtrees()])
    if (activeTab.value === 'history') await refreshHistory()
    ok = true
  } catch (e: any) {
    showToast(String(e))
  } finally {
    commitLoading.value = false
  }
  if (ok) commitSuccessTick.value++
}

async function softResetLastCommit() {
  if (!(await confirm('确认 soft reset 最后一次提交？最后一次提交会被撤销，改动保留在 Staged。'))) return
  try {
    const result = await invoke<string>('soft_reset_last_commit')
    showToast(!result || result === 'ok' ? '已 soft reset HEAD~1' : result, 'success')
    clearFileSelection()
    selectedCommitHash.value = null
    selectedCommitMsg.value = ''
    await Promise.all([syncRefresh({ silentStatus: true }), refreshHistory()])
  } catch (e: any) {
    showToast(String(e))
  }
}

// Select file - show diff
async function selectFile(
  path: string,
  isStaged: boolean,
  modifiers?: { shiftKey: boolean; toggleKey: boolean; sectionPaths: string[] },
) {
  const sectionPaths = modifiers?.sectionPaths ?? [path]
  const result = applyFileListSelectionClick({
    sectionPaths,
    current: selectedFilePaths.value,
    clickedPath: path,
    anchorPath: selectionAnchor.value,
    shiftKey: modifiers?.shiftKey ?? false,
    toggleKey: modifiers?.toggleKey ?? false,
  })
  selectedFilePaths.value = result.selected
  selectionAnchor.value = result.anchorPath

  let primary = path
  let primaryIsStaged = isStaged
  if (!result.selected.has(path)) {
    const remaining = [...result.selected]
    if (remaining.length === 0) {
      clearFileSelection()
      selectedCommitHash.value = null
      selectedCommitMsg.value = ''
      return
    }
    primary = remaining[remaining.length - 1]
    primaryIsStaged = statuses.value.some((s) => s.path === primary && s.isStaged)
  }

  selectedFile.value = primary
  selectedFileIsStaged.value = primaryIsStaged
  selectedCommitHash.value = null
  selectedCommitMsg.value = ''
  const ordered = sectionPaths.filter((item) => result.selected.has(item))
  await loadWorkspaceDiffs(ordered, primaryIsStaged)
}

async function refreshSelectedFileDiff() {
  await syncSelectedWorkspaceDiff()
  await refreshOperationState()
}

// Stage a patch (hunk or selected lines)
async function handleStagePatch(patch: string) {
  if (patchStaging.value) return
  patchStaging.value = true
  try {
    await invoke('stage_patch', { patch })
    await refreshStatus()
    await syncSelectedWorkspaceDiff()
    showToast('已 Stage', 'success')
  } catch (e: any) {
    showToast(String(e))
  } finally {
    patchStaging.value = false
  }
}

async function handleRevertPatch(patch: string, isStaged: boolean, revertedKeys: string[] = []) {
  if (patchStaging.value) return
  const msg = isStaged
    ? '确认丢弃选中的已 Stage 变更？此操作不可撤销。'
    : '确认丢弃选中的工作区变更？此操作不可撤销。'
  if (!(await confirm(msg))) return
  const targetFile = selectedFile.value
  const snapshot = diffText.value
  selectedFileIsStaged.value = isStaged
  if (revertedKeys.length > 0 && snapshot) {
    diffText.value = applyOptimisticRevertToDiffText(
      snapshot,
      targetFile,
      new Set(revertedKeys),
    )
  }
  patchStaging.value = true
  console.log('[revertPatch] isStaged=', isStaged, '\n--- patch start ---\n' + patch + '\n--- patch end ---')
  try {
    await invoke('revert_patch', { patch, isStaged })
    await refreshStatus()
    await syncSelectedWorkspaceDiff()
    showToast('已丢弃变更', 'success')
  } catch (e: any) {
    diffText.value = snapshot
    showToast(String(e))
  } finally {
    patchStaging.value = false
  }
}

// Select commit - show commit diff
async function selectCommit(hash: string) {
  const commit = commitLogs.value.find((c) => c.hash === hash)
  diffRequestSeq++
  selectedCommitHash.value = hash
  selectedCommitMsg.value = commit?.message ?? ''
  selectedFilePaths.value = new Set()
  selectionAnchor.value = null
  selectedFile.value = null
  selectedFileIsStaged.value = false
  try {
    diffText.value = await invoke<string>('get_commit_diff', { hash })
  } catch (e: any) {
    diffText.value = ''
    showToast(String(e))
  }
}

async function handleCommitAction(payload: { action: CommitAction; hash: string }) {
  const commit = commitLogs.value.find((item) => item.hash === payload.hash)
  const label = commit ? `${commit.message} (${payload.hash.slice(0, 7)})` : payload.hash.slice(0, 7)

  if (payload.action === 'cherryPick') {
    if (!(await confirm(`确认 cherry-pick ${label}？`))) return
    await runHistoryGitAction('cherry_pick_commit', { hash: payload.hash }, 'Cherry-pick 完成')
    return
  }

  if (payload.action === 'revert') {
    if (!(await confirm(`确认 revert ${label}？`))) return
    await runHistoryGitAction('revert_commit', { hash: payload.hash }, 'Revert 完成')
    return
  }

  const name = window.prompt(`从 ${payload.hash.slice(0, 7)} 创建分支：`)
  if (!name?.trim()) return
  await runHistoryGitAction(
    'create_branch_at',
    { name: name.trim(), hash: payload.hash },
    `分支「${name.trim()}」已创建`,
  )
}

async function runHistoryGitAction(command: string, args: Record<string, string>, successMessage: string) {
  try {
    const result = await invoke<string>(command, args)
    showToast(!result || result === 'ok' ? successMessage : result, 'success')
    await Promise.all([syncRefresh({ silentStatus: true }), refreshHistory()])
  } catch (e: any) {
    showToast(String(e))
    await Promise.all([refreshStatus({ silent: true }), refreshOperationState()])
  }
}

// Push / Pull
async function gitPush() {
  pushLoading.value = true
  try {
    const result = await invoke<string>('git_push')
    showToast(!result || result === 'ok' ? 'Push 成功' : result, 'success')
    await refreshAheadBehind()
  } catch (e: any) {
    showToast(String(e))
  } finally {
    pushLoading.value = false
  }
}

async function gitPull() {
  pullLoading.value = true
  try {
    const result = await invoke<string>('git_pull')
    showToast(!result || result === 'ok' ? 'Pull 成功' : result, 'success')
    await Promise.all([refreshStatus(), refreshAheadBehind(), refreshSubtrees(), refreshSubmodules(), refreshOperationState()])
  } catch (e: any) {
    showToast(String(e))
    await refreshOperationState()
  } finally {
    pullLoading.value = false
  }
}

async function subtreePull(prefix: string, remote: string, branch: string) {
  subtreeActionLoading.value = prefix
  try {
    const result = await invoke<string>('subtree_pull', { prefix, remote, branch })
    showToast(!result || result === 'ok' ? `Subtree pull 成功：${prefix}` : result, 'success')
    await syncRefresh({ silentStatus: true })
  } catch (e: any) {
    showToast(String(e))
  } finally {
    subtreeActionLoading.value = null
  }
}

async function subtreePush(prefix: string, remote: string, branch: string) {
  subtreeActionLoading.value = prefix
  try {
    const result = await invoke<string>('subtree_push', { prefix, remote, branch })
    showToast(!result || result === 'ok' ? `Subtree push 成功：${prefix}` : result, 'success')
    await syncRefresh({ silentStatus: true })
  } catch (e: any) {
    showToast(String(e))
  } finally {
    subtreeActionLoading.value = null
  }
}

async function updateSubmodule(path: string) {
  submoduleActionLoading.value = path
  try {
    const result = await invoke<string>('update_submodule', { path })
    showToast(!result || result === 'ok' ? `Submodule update 成功：${path}` : result, 'success')
    await syncRefresh({ silentStatus: true })
  } catch (e: any) {
    showToast(String(e))
  } finally {
    submoduleActionLoading.value = null
  }
}

// Ahead / behind
async function refreshAheadBehind() {
  if (!repoPath.value) return
  try {
    aheadBehind.value = await invoke<AheadBehind>('get_ahead_behind')
  } catch (_) {
    // no upstream configured — ignore
  }
}

// Checkout branch
const checkoutDialog = ref<{
  target: string
  isRemote: boolean
  state: WorktreeState
} | null>(null)

function worktreeNeedsAction(state: WorktreeState): boolean {
  return state.hasChanges || state.inMerge || state.inRebase || state.inCherryPick
}

function checkoutDialogSummary(state: WorktreeState): string {
  const parts: string[] = []
  if (state.inMerge) parts.push('进行中的合并')
  if (state.inRebase) parts.push('进行中的 Rebase')
  if (state.inCherryPick) parts.push('进行中的 Cherry-pick')
  if (state.hasChanges) parts.push('未提交的更改')
  return parts.join('、')
}

async function performCheckout(target: string, isRemote: boolean, mode: CheckoutMode) {
  try {
    await invoke('checkout_with_mode', { target, mode, isRemote })
    const label = isRemote ? target.split('/').pop() ?? target : target
    showToast(
      isRemote ? `已切换到 ${label}（跟踪 ${target}）` : `已切换到 ${label}`,
      'success',
    )
    clearFileSelection()
    selectedCommitHash.value = null
    checkoutDialog.value = null
    await Promise.all([syncRefresh(), stashList()])
  } catch (e: any) {
    showToast(String(e))
  }
}

async function requestCheckout(target: string, isRemote: boolean) {
  try {
    const state = await invoke<WorktreeState>('get_worktree_state')
    if (!worktreeNeedsAction(state)) {
      await performCheckout(target, isRemote, 'normal')
      return
    }
    checkoutDialog.value = { target, isRemote, state }
  } catch (e: any) {
    showToast(String(e))
  }
}

function cancelCheckoutDialog() {
  checkoutDialog.value = null
}

async function checkoutBranch(name: string) {
  await requestCheckout(name, false)
}

async function checkoutRemote(remote: string) {
  await requestCheckout(remote, true)
}

// Branch operations
async function renameBranch(oldName: string, newName: string) {
  try {
    await invoke('rename_branch', { oldName, newName })
    showToast(`分支已重命名为「${newName}」`, 'success')
    await Promise.all([refreshBranches(), refreshStatus()])
  } catch (e: any) {
    showToast(String(e))
  }
}

async function deleteBranch(name: string) {
  try {
    await invoke('delete_branch', { name, force: false })
    showToast(`分支「${name}」已删除`, 'success')
    await Promise.all([refreshBranches(), refreshStatus()])
  } catch (e: any) {
    // If normal delete fails, try force
    try {
      await invoke('delete_branch', { name, force: true })
      showToast(`分支「${name}」已强制删除`, 'success')
      await Promise.all([refreshBranches(), refreshStatus()])
    } catch (e2: any) {
      showToast(String(e2))
    }
  }
}

async function mergeBranch(name: string) {
  try {
    await invoke('merge_branch', { name })
    showToast(`已将「${name}」合并到当前分支`, 'success')
    await Promise.all([refreshStatus(), refreshAheadBehind(), refreshOperationState()])
  } catch (e: any) {
    showToast(String(e))
    await refreshOperationState()
  }
}

async function createBranch(name: string) {
  try {
    await invoke('create_branch', { name })
    showToast(`已切换到新分支「${name}」`, 'success')
    clearFileSelection()
    selectedCommitHash.value = null
    await Promise.all([refreshBranches(), refreshStatus(), refreshAheadBehind()])
  } catch (e: any) {
    showToast(String(e))
  }
}

async function continueOperation() {
  try {
    const result = await invoke<string>('continue_operation')
    showToast(!result || result === 'ok' ? '操作已继续' : result, 'success')
    await syncRefresh({ silentStatus: true })
    if (activeTab.value === 'history') await refreshHistory()
  } catch (e: any) {
    showToast(String(e))
    await refreshOperationState()
  }
}

async function abortOperation() {
  if (!(await confirm('确认中止当前 Git 操作？'))) return
  try {
    const result = await invoke<string>('abort_operation')
    showToast(!result || result === 'ok' ? '操作已中止' : result, 'success')
    clearFileSelection()
    selectedCommitHash.value = null
    await syncRefresh({ silentStatus: true })
  } catch (e: any) {
    showToast(String(e))
    await refreshOperationState()
  }
}

async function markFileResolved(path: string) {
  try {
    await invoke<string>('mark_file_resolved', { path })
    showToast(`已标记解决：${path}`, 'success')
    await syncRefresh({ silentStatus: true })
    await syncSelectedWorkspaceDiff()
  } catch (e: any) {
    showToast(String(e))
    await refreshOperationState()
  }
}

// Fetch
async function gitFetch() {
  fetchLoading.value = true
  try {
    const result = await invoke<string>('git_fetch')
    showToast(!result || result === 'ok' ? 'Fetch 完成' : result, 'success')
    await refreshAheadBehind()
  } catch (e: any) {
    showToast(String(e))
  } finally {
    fetchLoading.value = false
  }
}
// === Pin branches ===

async function pinBranch(branch: string) {
  try {
    await invoke('pin_branch', { branch })
    await refreshPinnedBranches()
  } catch (e: any) {
    showToast(String(e))
  }
}

async function unpinBranch(branch: string) {
  try {
    await invoke('unpin_branch', { branch })
    await refreshPinnedBranches()
  } catch (e: any) {
    showToast(String(e))
  }
}

// === Tag ===

async function createTag(name: string, message?: string) {
  try {
    await invoke('create_tag', { name, message: message || null })
    showToast('标签 ' + name + ' 已创建', 'success')
    await refreshTags()
  } catch (e: any) {
    showToast(String(e))
  }
}

// === Stash ===

async function stashSave(message: string | null, includeUntracked: boolean) {
  try {
    await invoke('stash_save', { message, includeUntracked })
    showToast('已 Stash', 'success')
    await Promise.all([stashList(), refreshStatus()])
  } catch (e: any) {
    showToast(String(e))
  }
}

async function stashList() {
  try {
    stashEntries.value = await invoke('stash_list')
  } catch (e: any) {
    showToast(String(e))
    stashEntries.value = []
  }
}

async function stashApply(index: number) {
  try {
    await invoke('stash_apply', { index })
    showToast('已恢复 stash@{' + index + '}', 'success')
    await syncRefresh()
  } catch (e: any) {
    showToast(String(e))
  }
}

async function stashDrop(index: number) {
  try {
    await invoke('stash_drop', { index })
    showToast('已删除 stash@{' + index + '}', 'success')
    await stashList()
  } catch (e: any) {
    showToast(String(e))
  }
}



// History
async function fetchHistoryPage(skip: number): Promise<CommitLogPage> {
  return invoke<CommitLogPage>('get_git_log', {
    all: historyFilter.value === 'all',
    skip,
    limit: HISTORY_PAGE_SIZE,
  })
}

async function refreshHistory() {
  if (!repoPath.value) return
  const requestId = ++historyRequestSeq
  historyLoading.value = true
  try {
    await refreshAheadBehind()
    const page = await fetchHistoryPage(0)
    if (requestId !== historyRequestSeq) return
    commitLogs.value = page.commits
    historyHasMore.value = page.hasMore
  } catch (e: any) {
    showToast(String(e))
  } finally {
    if (requestId === historyRequestSeq) historyLoading.value = false
  }
}

async function loadMoreHistory() {
  if (!repoPath.value || !historyHasMore.value || historyLoadingMore.value || historyLoading.value) return
  historyLoadingMore.value = true
  try {
    const page = await fetchHistoryPage(commitLogs.value.length)
    commitLogs.value = [...commitLogs.value, ...page.commits]
    historyHasMore.value = page.hasMore
  } catch (e: any) {
    showToast(String(e))
  } finally {
    historyLoadingMore.value = false
  }
}

// Tab switch
async function onSwitchTab(tab: 'workspace' | 'history') {
  activeTab.value = tab
  if (tab === 'history') {
    await refreshHistory()
  } else {
    // Refresh workspace state（外部改文件 / 命令行 git 后回到工作区应立刻对齐，不必整页 loading）
    await syncRefresh({ silentStatus: true })
    // Clear commit selection when switching back to workspace
    selectedCommitHash.value = null
    selectedCommitMsg.value = ''
    await syncSelectedWorkspaceDiff()
  }
}

const selectedFilesForPanel = computed(() => [...selectedFilePaths.value])
</script>

<template>
  <Splitpanes class="h-full w-full bg-[--bg-primary]">
    <!-- Sidebar -->
    <Pane :min-size="15" :max-size="25" :size="18">
      <SidebarPanel
        :repo-path="repoPath"
        :active-tab="activeTab"
        :push-loading="pushLoading"
        :pull-loading="pullLoading"
        :branches="branches"
        :branches-loading="branchesLoading"
        :recent-repos="recentRepos"
        :ahead-behind="aheadBehind"
        :fetch-loading="fetchLoading"
        @open-repo="openRepo"
        @switch-repo="switchRepo"
        @remove-recent-repo="removeRecentRepo"
        @reveal-error="showToast($event)"
        @switch-tab="onSwitchTab"
        @checkout-branch="checkoutBranch"
        @checkout-remote="checkoutRemote"
        @rename-branch="renameBranch"
        @delete-branch="deleteBranch"
        @merge-branch="mergeBranch"
        @create-branch="createBranch"
        :pinned-branches="pinnedBranches"
        :stash-entries="stashEntries"
        :tags="tags"
        :subtrees="subtrees"
        :submodules="submodules"
        :subtree-action-loading="subtreeActionLoading"
        :submodule-action-loading="submoduleActionLoading"
        @pin-branch="pinBranch"
        @unpin-branch="unpinBranch"
        @create-tag="createTag"
        @stash-save="stashSave"
        @stash-list="stashList"
        @tags-list="refreshTags"
        @stash-apply="stashApply"
        @stash-drop="stashDrop"
        @fetch="gitFetch"
        @push="gitPush"
        @pull="gitPull"
        @subtree-pull="subtreePull"
        @subtree-push="subtreePush"
        @update-submodule="updateSubmodule"
        @settings-open="settingsOpen = true"
        @clone-repo="cloneRepo"
      />
    </Pane>

    <!-- Middle panel -->
    <Pane :min-size="28">
      <div class="h-full min-h-0 flex flex-col">
        <div
          v-if="operationActive"
          class="shrink-0 flex items-center gap-2 px-3 py-2 border-b border-amber-500/30 bg-amber-500/10 text-xs"
        >
          <span class="font-medium text-amber-700 dark:text-amber-300">{{ operationLabel }}</span>
          <span v-if="operationState.hasConflicts" class="text-[--text-secondary] truncate">
            {{ operationState.conflictedFiles.join(', ') }}
          </span>
          <div class="flex-1" />
          <button
            class="px-2 py-1 rounded-[var(--radius)] text-xs bg-[--accent] text-white hover:bg-[--accent-hover] transition-colors cursor-pointer disabled:opacity-40 disabled:cursor-not-allowed"
            :disabled="!operationState.canContinue"
            @click="continueOperation"
          >
            Continue
          </button>
          <button
            class="px-2 py-1 rounded-[var(--radius)] text-xs text-red-300 border border-red-800/60 hover:bg-red-900/40 transition-colors cursor-pointer disabled:opacity-40 disabled:cursor-not-allowed"
            :disabled="!operationState.canAbort"
            @click="abortOperation"
          >
            Abort
          </button>
        </div>

        <div class="flex-1 min-h-0">
          <!-- Workspace tab -->
          <WorkspacePanel
            v-if="activeTab === 'workspace'"
            :statuses="statuses"
            :selected-file="selectedFile"
            :selected-files="selectedFilesForPanel"
            :commit-loading="commitLoading"
            :commit-success-tick="commitSuccessTick"
            :status-loading="statusLoading"
            :repo-path="repoPath"
            :settings-revision="settingsRevision"
            :subtrees="subtrees"
            :conflicted-files="operationState.conflictedFiles"
            @stage-file="stageFile"
            @unstage-file="unstageFile"
            @revert-file="revertFile"
            @delete-file="deleteFile"
            @select-file="selectFile"
            @commit="commitChanges"
            @soft-reset-last-commit="softResetLastCommit"
            @reveal-error="showToast($event)"
            @open-settings="settingsOpen = true"
          />
          <!-- History tab -->
          <HistoryTab
            v-if="activeTab === 'history'"
            :logs="commitLogs"
            :loading="historyLoading"
            :loading-more="historyLoadingMore"
            :has-more="historyHasMore"
            :selected-hash="selectedCommitHash"
            :filter="historyFilter"
            :current-branch="currentBranch"
            :branch-tips="branchTips"
            :unpushed-hashes="aheadBehind.unpushedHashes"
            @select-commit="selectCommit"
            @commit-action="handleCommitAction"
            @update-filter="historyFilter = $event; refreshHistory()"
            @load-more="loadMoreHistory"
          />
        </div>
      </div>
    </Pane>

    <!-- Diff panel -->
    <Pane :min-size="30" :size="42">
      <DiffPanel
        :diff-text="diffText"
        :file-name="diffFileName"
        :can-stage="!selectedCommitHash && !!selectedFile && !selectedFileIsStaged"
        :can-revert="!selectedCommitHash && !!selectedFile"
        :file-path="selectedCommitHash ? null : selectedFile"
        :repo-path="repoPath"
        :workspace-is-staged="selectedFileIsStaged"
        :commit-hash="selectedCommitHash"
        :patch-staging="patchStaging"
        :conflicted="selectedFilesForPanel.length === 1 && !!selectedFile && operationState.conflictedFiles.includes(selectedFile)"
        @stage-patch="handleStagePatch"
        @stage-file="stageFile"
        @revert-patch="handleRevertPatch"
        @revert-file="revertFile"
        @mark-resolved="markFileResolved"
        @conflict-file-updated="refreshSelectedFileDiff"
      />
    </Pane>
  </Splitpanes>

    <!-- Drag-and-drop overlay -->
    <Transition name="toast">
      <div
        v-if="repoDragOver"
        class="fixed inset-0 z-[10000] flex items-center justify-center bg-[--accent]/10 border-2 border-dashed border-[--accent]/60 pointer-events-none"
      >
        <div class="px-4 py-3 rounded-[var(--radius)] bg-[--bg-tertiary] border border-[--border-color] shadow-lg text-sm text-[--text-primary]">
          拖放 Git 仓库目录到此处
        </div>
      </div>
    </Transition>

    <!-- Global loading indicator -->
    <div
      v-if="globalRefreshing"
      class="fixed top-2 left-1/2 -translate-x-1/2 z-[9999] flex items-center gap-1.5 px-2 py-1 rounded-full bg-[--accent] shadow-md"
    >
      <Loader2 :size="10" class="animate-spin text-white" />
      <span class="text-[10px] text-white">同步中...</span>
    </div>

    <!-- Toast -->
    <Transition name="toast">
      <div
        v-if="toast"
        class="fixed bottom-2.5 right-2.5 z-[9999] px-2.5 py-2.5 rounded shadow-lg text-xs max-w-sm break-words"
        :class="toast.type === 'error'
          ? 'bg-red-800 text-red-100 border border-red-700'
          : 'bg-green-800 text-green-100 border border-green-700'"
      >
        {{ toast.message }}
      </div>
    </Transition>

    <!-- Settings Dialog -->
    <SettingsDialog
      :show="settingsOpen"
      @close="settingsOpen = false"
      @saved="settingsRevision++"
    />

    <!-- Checkout conflict dialog -->
    <Teleport to="body">
      <div
        v-if="checkoutDialog"
        class="fixed inset-0 z-[9999] flex items-center justify-center bg-black/40"
        @click="cancelCheckoutDialog"
      >
        <div
          class="bg-[--bg-tertiary] border border-[--border-color] rounded-[var(--radius)] shadow-xl p-2.5 w-[320px]"
          @click.stop
        >
          <div class="text-xs text-[--text-primary] mb-2 font-medium">切换分支前需处理工作区</div>
          <div class="text-[10px] text-[--text-secondary] mb-1">
            目标：{{ checkoutDialog.isRemote ? checkoutDialog.target.split('/').pop() : checkoutDialog.target }}
          </div>
          <div class="text-[10px] text-[--text-secondary] mb-2.5">
            当前有 {{ checkoutDialogSummary(checkoutDialog.state) }}
          </div>
          <div class="flex flex-col gap-1.5">
            <button
              v-if="!checkoutDialog.state.inMerge && !checkoutDialog.state.inRebase && !checkoutDialog.state.inCherryPick"
              class="w-full px-2.5 py-2.5 rounded-[var(--radius)] text-xs bg-[--accent] text-white hover:bg-[--accent-hover] transition-colors cursor-pointer text-left"
              @click="performCheckout(checkoutDialog.target, checkoutDialog.isRemote, 'stash')"
            >
              Stash 并切换
              <span class="block text-[10px] opacity-80 mt-0.5">暂存当前更改后切换分支</span>
            </button>
            <button
              class="w-full px-2.5 py-2.5 rounded-[var(--radius)] text-xs text-red-300 border border-red-800/60 hover:bg-red-900/40 transition-colors cursor-pointer text-left"
              @click="performCheckout(checkoutDialog.target, checkoutDialog.isRemote, 'discard')"
            >
              丢弃更改并切换
              <span class="block text-[10px] opacity-80 mt-0.5">
                放弃所有本地更改
                <template v-if="checkoutDialog.state.inMerge || checkoutDialog.state.inRebase || checkoutDialog.state.inCherryPick">
                  ，并中止进行中的合并/Rebase
                </template>
              </span>
            </button>
            <button
              class="w-full px-2.5 py-2.5 rounded-[var(--radius)] text-xs text-[--text-secondary] hover:text-[--text-primary] hover:bg-[--bg-secondary] transition-colors cursor-pointer"
              @click="cancelCheckoutDialog"
            >
              取消
            </button>
          </div>
        </div>
      </div>
    </Teleport>
</template>
