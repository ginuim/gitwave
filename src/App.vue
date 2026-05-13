<script setup lang="ts">
import { ref, computed, onMounted } from 'vue'
import { invoke } from '@tauri-apps/api/core'
import { Splitpanes, Pane } from 'splitpanes'
import 'splitpanes/dist/splitpanes.css'
import SidebarPanel from './components/SidebarPanel.vue'
import WorkspacePanel from './components/WorkspacePanel.vue'
import DiffPanel from './components/DiffPanel.vue'
import HistoryTab from './components/HistoryTab.vue'
import type { FileStatus, CommitLog, BranchInfo } from './types'

// State
const repoPath = ref<string | null>(null)
const activeTab = ref<'workspace' | 'history'>('workspace')
const statuses = ref<FileStatus[]>([])
const selectedFile = ref<string | null>(null)
const selectedCommitHash = ref<string | null>(null)
const selectedCommitMsg = ref('')
const diffText = ref('')
const commitLogs = ref<CommitLog[]>([])
const diffFileName = computed(() => {
  if (selectedCommitHash.value) return `commit ${selectedCommitHash.value} - ${selectedCommitMsg.value}`
  return selectedFile.value
})

const branches = ref<BranchInfo[]>([])
const branchesLoading = ref(false)

const statusLoading = ref(false)
const commitLoading = ref(false)
const historyLoading = ref(false)
const pushPullLoading = ref(false)

// Toast
const toast = ref<{ message: string; type: 'error' | 'success' } | null>(null)
let toastTimer: ReturnType<typeof setTimeout> | null = null

function showToast(message: string, type: 'error' | 'success' = 'error') {
  toast.value = { message, type }
  if (toastTimer) clearTimeout(toastTimer)
  toastTimer = setTimeout(() => {
    toast.value = null
  }, 4000)
}

// Open repository
async function openRepo() {
  try {
    const path = await invoke<string>('open_repository')
    repoPath.value = path
    showToast('仓库已打开', 'success')
    await Promise.all([refreshStatus(), refreshBranches()])
  } catch (e: any) {
    if (e !== 'dialog cancelled') {
      showToast(String(e))
    }
  }
}

// Get repo path on mount
onMounted(async () => {
  try {
    const path = await invoke<string | null>('get_repo_path')
    repoPath.value = path
    if (path) {
      await Promise.all([refreshStatus(), refreshBranches()])
    }
  } catch (_) {
    // ignore
  }
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

// Refresh status
async function refreshStatus() {
  if (!repoPath.value) return
  statusLoading.value = true
  try {
    statuses.value = await invoke<FileStatus[]>('get_git_status')
    // Clear selection if selected file no longer exists
    const paths = statuses.value.map((s) => s.path)
    if (selectedFile.value && !paths.includes(selectedFile.value)) {
      selectedFile.value = null
      diffText.value = ''
    }
  } catch (e: any) {
    showToast(String(e))
  } finally {
    statusLoading.value = false
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
    await refreshStatus()
  } catch (e: any) {
    showToast(String(e))
  } finally {
    commitLoading.value = false
  }
}

// Select file - show diff
async function selectFile(path: string, isStaged: boolean) {
  selectedFile.value = path
  selectedCommitHash.value = null
  selectedCommitMsg.value = ''
  try {
    diffText.value = await invoke<string>('get_file_diff', { path, isStaged })
  } catch (e: any) {
    diffText.value = ''
    showToast(String(e))
  }
}

// Select commit - show commit diff
async function selectCommit(hash: string) {
  const commit = commitLogs.value.find((c) => c.hash === hash)
  selectedCommitHash.value = hash
  selectedCommitMsg.value = commit?.message ?? ''
  selectedFile.value = null
  try {
    diffText.value = await invoke<string>('get_commit_diff', { hash })
  } catch (e: any) {
    diffText.value = ''
    showToast(String(e))
  }
}

// Push / Pull
async function gitPush() {
  pushPullLoading.value = true
  try {
    const result = await invoke<string>('git_push')
    showToast(result === 'ok' ? 'Push 成功' : result, 'success')
  } catch (e: any) {
    showToast(String(e))
  } finally {
    pushPullLoading.value = false
  }
}

async function gitPull() {
  pushPullLoading.value = true
  try {
    const result = await invoke<string>('git_pull')
    showToast(result === 'ok' ? 'Pull 成功' : result, 'success')
    await refreshStatus()
  } catch (e: any) {
    showToast(String(e))
  } finally {
    pushPullLoading.value = false
  }
}

// History
async function refreshHistory() {
  if (!repoPath.value) return
  historyLoading.value = true
  try {
    commitLogs.value = await invoke<CommitLog[]>('get_git_log')
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
    // Clear commit selection when switching back to workspace
    selectedCommitHash.value = null
    selectedCommitMsg.value = ''
    if (!selectedFile.value) {
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
        :push-pull-loading="pushPullLoading"
        :branches="branches"
        :branches-loading="branchesLoading"
        @open-repo="openRepo"
        @switch-tab="onSwitchTab"
        @push="gitPush"
        @pull="gitPull"
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
        @stage-file="stageFile"
        @unstage-file="unstageFile"
        @select-file="selectFile"
        @commit="commitChanges"
      />
      <!-- History tab -->
      <HistoryTab
        v-if="activeTab === 'history'"
        :logs="commitLogs"
        :loading="historyLoading"
        :selected-hash="selectedCommitHash"
        @select-commit="selectCommit"
      />
    </Pane>

    <!-- Diff panel -->
    <Pane :min-size="30" :size="42">
      <DiffPanel
        :diff-text="diffText"
        :file-name="diffFileName"
      />
    </Pane>
  </Splitpanes>

    <!-- Toast -->
    <div
      v-if="toast"
      class="fixed bottom-4 right-4 z-50 px-4 py-2 rounded shadow-lg text-xs max-w-md break-words"
      :class="toast.type === 'error'
        ? 'bg-red-800 text-red-100 border border-red-700'
        : 'bg-green-800 text-green-100 border border-green-700'"
    >
      {{ toast.message }}
  </div>
</template>
