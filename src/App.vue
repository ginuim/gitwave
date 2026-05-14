<script setup lang="ts">
import { ref, computed, onMounted, onUnmounted } from 'vue'
import { invoke } from '@tauri-apps/api/core'
import { Splitpanes, Pane } from 'splitpanes'
import 'splitpanes/dist/splitpanes.css'
import SidebarPanel from './components/SidebarPanel.vue'
import WorkspacePanel from './components/WorkspacePanel.vue'
import DiffPanel from './components/DiffPanel.vue'
import HistoryTab from './components/HistoryTab.vue'
import SettingsDialog from './components/SettingsDialog.vue'
import type { FileStatus, CommitLog, BranchInfo, AheadBehind } from './types'

// State
const repoPath = ref<string | null>(null)
const activeTab = ref<'workspace' | 'history'>('workspace')
const statuses = ref<FileStatus[]>([])
const selectedFile = ref<string | null>(null)
/** 工作区列表里当前选中项是否在「已暂存」区域（用于二进制图片 diff 预览取哪一侧） */
const selectedFileIsStaged = ref(false)
const selectedCommitHash = ref<string | null>(null)
const selectedCommitMsg = ref('')
const diffText = ref('')
const commitLogs = ref<CommitLog[]>([])
const diffFileName = computed(() => {
  if (selectedCommitHash.value) return `commit ${selectedCommitHash.value} - ${selectedCommitMsg.value}`
  return selectedFile.value
})

const canStage = computed(() => !selectedCommitHash.value && !!selectedFile.value)

const branches = ref<BranchInfo[]>([])
const branchesLoading = ref(false)
const recentRepos = ref<string[]>([])
const pinnedBranches = ref<string[]>([])
const stashEntries = ref<any[]>([])
const settingsOpen = ref(false)
const settingsRevision = ref(0)

const historyFilter = ref<'current' | 'all'>('current')
const currentBranch = computed(() => branches.value.find(b => b.isCurrent)?.name ?? '')

const statusLoading = ref(false)
const commitLoading = ref(false)
const historyLoading = ref(false)
const pushLoading = ref(false)
const pullLoading = ref(false)
const aheadBehind = ref<AheadBehind>({ ahead: 0, behind: 0 })
const fetchLoading = ref(false)

// Toast
const toast = ref<{ message: string; type: 'error' | 'success' } | null>(null)
let toastTimer: ReturnType<typeof setTimeout> | null = null

function showToast(message: string, type: 'error' | 'success' = 'error') {
  toast.value = { message, type }
  if (toastTimer) clearTimeout(toastTimer)
  toastTimer = setTimeout(() => {
    toast.value = null
  }, 6000)
}

// Open repository
async function openRepo() {
  try {
    const path = await invoke<string>('open_repository')
    repoPath.value = path
    showToast('仓库已打开', 'success')
    await Promise.all([syncRefresh(), refreshRecentRepos()])
  } catch (e: any) {
    if (e !== 'dialog cancelled') {
      showToast(String(e))
    }
  }
}

function refreshWorkspaceIfVisible() {
  if (!repoPath.value) return
  void syncRefresh({ silentStatus: true })
}

// Get repo path on mount
onMounted(async () => {
  try {
    const path = await invoke<string | null>('get_repo_path')
    repoPath.value = path
    if (path) {
      await Promise.all([refreshStatus(), refreshBranches(), refreshAheadBehind()])
    }
  } catch (_) {
    // ignore
  }
  // Always load recent repos regardless of current session state
  await refreshRecentRepos()
  await refreshPinnedBranches()

  window.addEventListener('focus', refreshWorkspaceIfVisible)
  document.addEventListener('visibilitychange', refreshWorkspaceIfVisible)
})

onUnmounted(() => {
  window.removeEventListener('focus', refreshWorkspaceIfVisible)
  document.removeEventListener('visibilitychange', refreshWorkspaceIfVisible)
})

// Refresh branches
async function refreshBranches() {
  if (!repoPath.value) return
  branchesLoading.value = true
  try {
    branches.value = await invoke<BranchInfo[]>('get_branches')
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

async function refreshPinnedBranches() {
  try {
    pinnedBranches.value = await invoke<string[]>('get_pinned_branches')
  } catch (_) {
    // ignore
  }
}

async function syncRefresh(opts?: { silentStatus?: boolean }) {
  const tasks = [
    refreshStatus(opts?.silentStatus ? { silent: true } : undefined),
    refreshBranches(),
    refreshAheadBehind(),
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
  } catch (e: any) {
    showToast(String(e))
  }
}

// Refresh status（silent：后台同步，不挡整个列表的加载态）
async function refreshStatus(opts?: { silent?: boolean }) {
  if (!repoPath.value) return
  const silent = opts?.silent ?? false
  if (!silent) statusLoading.value = true
  try {
    statuses.value = await invoke<FileStatus[]>('get_git_status')
    // Clear selection if selected file no longer exists
    const paths = statuses.value.map((s) => s.path)
    if (selectedFile.value && !paths.includes(selectedFile.value)) {
      selectedFile.value = null
      diffText.value = ''
    }
  } catch (e: any) {
    if (!silent) showToast(String(e))
  } finally {
    if (!silent) statusLoading.value = false
  }
}

// Stage / Unstage
async function stageFile(path: string) {
  try {
    await invoke('stage_file', { path })
    await refreshStatus()
  } catch (e: any) {
    showToast(String(e))
  }
}

async function unstageFile(path: string) {
  try {
    await invoke('unstage_file', { path })
    await refreshStatus()
  } catch (e: any) {
    showToast(String(e))
  }
}

// Commit
async function commitChanges(message: string) {
  commitLoading.value = true
  try {
    await invoke('commit_changes', { message })
    showToast('提交成功', 'success')
    await Promise.all([refreshStatus(), refreshAheadBehind()])
  } catch (e: any) {
    showToast(String(e))
  } finally {
    commitLoading.value = false
  }
}

// Select file - show diff
async function selectFile(path: string, isStaged: boolean) {
  selectedFile.value = path
  selectedFileIsStaged.value = isStaged
  selectedCommitHash.value = null
  selectedCommitMsg.value = ''
  try {
    diffText.value = await invoke<string>('get_file_diff', { path, isStaged })
  } catch (e: any) {
    diffText.value = ''
    showToast(String(e))
  }
}

// Stage a patch (hunk or selected lines)
async function handleStagePatch(patch: string) {
  try {
    await invoke('stage_patch', { patch })
    showToast('已暂存', 'success')
    await refreshStatus()
    // Re-fetch diff to reflect staged state
    if (selectedFile.value) {
      diffText.value = await invoke<string>('get_file_diff', {
        path: selectedFile.value,
        isStaged: false,
      })
    }
  } catch (e: any) {
    showToast(String(e))
  }
}

// Select commit - show commit diff
async function selectCommit(hash: string) {
  const commit = commitLogs.value.find((c) => c.hash === hash)
  selectedCommitHash.value = hash
  selectedCommitMsg.value = commit?.message ?? ''
  selectedFile.value = null
  selectedFileIsStaged.value = false
  try {
    diffText.value = await invoke<string>('get_commit_diff', { hash })
  } catch (e: any) {
    diffText.value = ''
    showToast(String(e))
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
    await Promise.all([refreshStatus(), refreshAheadBehind()])
  } catch (e: any) {
    showToast(String(e))
  } finally {
    pullLoading.value = false
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
async function checkoutBranch(name: string) {
  try {
    await invoke('checkout_branch', { name })
    showToast(`已切换到 ${name}`, 'success')
    await syncRefresh()
  } catch (e: any) {
    showToast(String(e))
  }
}

// Checkout remote branch as local tracking branch
async function checkoutRemote(remote: string) {
  try {
    await invoke('checkout_remote_branch', { remote })
    const local = remote.split('/').pop()
    showToast(`已切换到 ${local}（跟踪 ${remote}）`, 'success')
    await syncRefresh()
  } catch (e: any) {
    showToast(String(e))
  }
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
    await Promise.all([refreshStatus(), refreshAheadBehind()])
  } catch (e: any) {
    showToast(String(e))
  }
}

async function createBranch(name: string) {
  try {
    await invoke('create_branch', { name })
    showToast(`已切换到新分支「${name}」`, 'success')
    selectedFile.value = null
    selectedCommitHash.value = null
    diffText.value = ''
    await Promise.all([refreshBranches(), refreshStatus(), refreshAheadBehind()])
  } catch (e: any) {
    showToast(String(e))
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
  } catch (e: any) {
    showToast(String(e))
  }
}

// === Stash ===

async function stashSave(message: string | null, includeUntracked: boolean) {
  try {
    await invoke('stash_save', { message, includeUntracked })
    showToast('已暂存', 'success')
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
  } catch (e: any) {
    showToast(String(e))
  }
}



// History
async function refreshHistory() {
  if (!repoPath.value) return
  historyLoading.value = true
  try {
    commitLogs.value = await invoke<CommitLog[]>('get_git_log', { all: historyFilter.value === 'all' })
  } catch (e: any) {
    showToast(String(e))
  } finally {
    historyLoading.value = false
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
    // Re-fetch diff for the selected file if still relevant
    if (selectedFile.value && statuses.value.some(s => s.path === selectedFile.value)) {
      try {
        diffText.value = await invoke<string>('get_file_diff', {
          path: selectedFile.value,
          isStaged: false,
        })
      } catch {
        diffText.value = ''
      }
    } else {
      selectedFile.value = null
      diffText.value = ''
    }
  }
}
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
        @switch-tab="onSwitchTab"
        @checkout-branch="checkoutBranch"
        @checkout-remote="checkoutRemote"
        @rename-branch="renameBranch"
        @delete-branch="deleteBranch"
        @merge-branch="mergeBranch"
        @create-branch="createBranch"
        :pinned-branches="pinnedBranches"
        :stash-entries="stashEntries"
        @pin-branch="pinBranch"
        @unpin-branch="unpinBranch"
        @create-tag="createTag"
        @stash-save="stashSave"
        @stash-list="stashList"
        @stash-apply="stashApply"
        @stash-drop="stashDrop"
        @fetch="gitFetch"
        @push="gitPush"
        @pull="gitPull"
        @settings-open="settingsOpen = true"
      />
    </Pane>

    <!-- Middle panel -->
    <Pane :min-size="25">
      <!-- Workspace tab -->
      <WorkspacePanel
        v-if="activeTab === 'workspace'"
        :statuses="statuses"
        :selected-file="selectedFile"
        :commit-loading="commitLoading"
        :status-loading="statusLoading"
        :repo-path="repoPath"
        :settings-revision="settingsRevision"
        @stage-file="stageFile"
        @unstage-file="unstageFile"
        @select-file="selectFile"
        @commit="commitChanges"
        @reveal-error="showToast($event)"
      />
      <!-- History tab -->
      <HistoryTab
        v-if="activeTab === 'history'"
        :logs="commitLogs"
        :loading="historyLoading"
        :selected-hash="selectedCommitHash"
        :filter="historyFilter"
        :current-branch="currentBranch"
        @select-commit="selectCommit"
        @update-filter="historyFilter = $event; refreshHistory()"
      />
    </Pane>

    <!-- Diff panel -->
    <Pane :min-size="30" :size="42">
      <DiffPanel
        :diff-text="diffText"
        :file-name="diffFileName"
        :can-stage="!selectedCommitHash && !!selectedFile"
        :file-path="canStage ? selectedFile : null"
        :repo-path="repoPath"
        :workspace-is-staged="selectedFileIsStaged"
        :commit-hash="selectedCommitHash"
        @stage-patch="handleStagePatch"
        @stage-file="stageFile"
      />
    </Pane>
  </Splitpanes>

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
</template>
