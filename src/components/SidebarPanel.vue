<script setup lang="ts">
import { ref, computed } from 'vue'
import {
  FolderOpen, GitBranch, Globe,
  List, History, Loader2, ChevronRight, ChevronDown, Plus,
  ChevronDown as ChevronDownIcon,
  RefreshCw, ArrowDownToLine, ArrowUpToLine,
} from 'lucide-vue-next'
import type { BranchInfo, AheadBehind } from '../types'

const props = defineProps<{
  repoPath: string | null
  activeTab: 'workspace' | 'history'
  pushLoading: boolean
  pullLoading: boolean
  branches: BranchInfo[]
  branchesLoading: boolean
  recentRepos: string[]
  aheadBehind: AheadBehind
  fetchLoading: boolean
}>()

const emit = defineEmits<{
  openRepo: []
  switchRepo: [path: string]
  switchTab: [tab: 'workspace' | 'history']
  checkoutBranch: [name: string]
  checkoutRemote: [remote: string]
  renameBranch: [oldName: string, newName: string]
  deleteBranch: [name: string]
  mergeBranch: [name: string]
  createBranch: [name: string]
  fetch: []
  push: []
  pull: []
}>()

// --- Dropdown ---
const dropdownOpen = ref(false)

function toggleDropdown() {
  dropdownOpen.value = !dropdownOpen.value
}

function selectPath(path: string) {
  dropdownOpen.value = false
  emit('switchRepo', path)
}

function selectOpenOther() {
  dropdownOpen.value = false
  emit('openRepo')
}

function handleClickOutside(e: MouseEvent) {
  const target = e.target as HTMLElement
  if (!target.closest('.repo-dropdown')) {
    dropdownOpen.value = false
  }
}

// --- Context menu ---
const ctxMenu = ref<{ x: number; y: number; branch: string; isCurrent: boolean } | null>(null)

function handleContextMenu(e: MouseEvent, branch: string, isCurrent: boolean) {
  e.preventDefault()
  ctxMenu.value = { x: e.clientX, y: e.clientY, branch, isCurrent }
}

function closeCtxMenu() {
  ctxMenu.value = null
}

function dirName(path: string): string {
  const parts = path.replace(/\\/g, '/').split('/')
  return parts[parts.length - 1] || path
}

// --- Create branch dialog ---
const createBranchDialog = ref(false)
const newBranchName = ref('')
const createBranchInput = ref<HTMLInputElement | null>(null)

function openCreateBranch() {
  createBranchDialog.value = true
  newBranchName.value = ''
  setTimeout(() => createBranchInput.value?.focus(), 50)
}

function submitCreateBranch() {
  const name = newBranchName.value.trim()
  if (!name) return
  emit('createBranch', name)
  createBranchDialog.value = false
  newBranchName.value = ''
}

function cancelCreateBranch() {
  createBranchDialog.value = false
  newBranchName.value = ''
}

// --- Rename dialog ---
const renameDialog = ref<{ branch: string; value: string } | null>(null)
const renameInput = ref<HTMLInputElement | null>(null)

function ctxRename() {
  const b = ctxMenu.value?.branch
  closeCtxMenu()
  if (!b) return
  renameDialog.value = { branch: b, value: b }
  // focus input after next tick
  setTimeout(() => renameInput.value?.focus(), 50)
}

function submitRename() {
  const d = renameDialog.value
  if (!d) return
  if (d.value && d.value !== d.branch) {
    emit('renameBranch', d.branch, d.value)
  }
  renameDialog.value = null
}

function cancelRename() {
  renameDialog.value = null
}

function ctxDelete() {
  const b = ctxMenu.value?.branch
  closeCtxMenu()
  if (!b) return
  if (confirm(`确认删除分支「${b}」？`)) {
    emit('deleteBranch', b)
  }
}

function ctxMerge() {
  const b = ctxMenu.value?.branch
  closeCtxMenu()
  if (!b) return
  if (confirm(`确认将「${b}」合并到当前分支？`)) {
    emit('mergeBranch', b)
  }
}

// Close context menu on outside click
function globalClick() {
  closeCtxMenu()
  // also close dropdown
  dropdownOpen.value = false
}

// --- Branch tree logic ---

interface TreeNode {
  label: string
  depth: number
  isLeaf: boolean
  isCurrent: boolean
  isRemote: boolean
  isHead: boolean
  key: string
}

const expandedGroups = ref<Set<string>>(new Set())

function toggleGroup(key: string) {
  const s = expandedGroups.value
  if (s.has(key)) s.delete(key)
  else s.add(key)
  expandedGroups.value = new Set(s)
}

function buildTree(branches: BranchInfo[]): TreeNode[] {
  const root: Record<string, any> = {}

  for (const b of branches) {
    const parts = b.name.split('/')
    let node = root
    for (let i = 0; i < parts.length; i++) {
      const part = parts[i]
      if (i === parts.length - 1) {
        node[part] = { __leaf: true, branch: b }
      } else {
        if (!node[part]) node[part] = {}
        node = node[part]
      }
    }
  }

  function flatten(obj: Record<string, any>, depth: number, prefix: string): TreeNode[] {
    const result: TreeNode[] = []
    const keys = Object.keys(obj).sort()
    for (const key of keys) {
      const val = obj[key]
      if (val.__leaf) {
        const b = val.branch as BranchInfo
        result.push({
          label: key,
          depth,
          isLeaf: true,
          isCurrent: b.isCurrent,
          isRemote: b.isRemote,
          isHead: b.isHead,
          key: prefix + key,
        })
      } else {
        const groupKey = prefix + key + '/'
        const hasLeaves = hasLeafDescendant(val)
        result.push({
          label: key,
          depth,
          isLeaf: false,
          isCurrent: false,
          isRemote: false,
          isHead: false,
          key: groupKey,
        })
        if (expandedGroups.value.has(groupKey) && hasLeaves) {
          result.push(...flatten(val, depth + 1, groupKey))
        }
      }
    }
    return result
  }

  return flatten(root, 0, '')
}

function hasLeafDescendant(obj: Record<string, any>): boolean {
  for (const key of Object.keys(obj)) {
    const val = obj[key]
    if (val.__leaf) return true
    if (hasLeafDescendant(val)) return true
  }
  return false
}

const localBranches = computed(() => buildTree(props.branches.filter(b => !b.isRemote)))
const remoteBranches = computed(() => buildTree(props.branches.filter(b => b.isRemote)))
</script>

<template>
  <div
    class="h-full flex flex-col bg-[--bg-secondary] border-r border-[--border-color] select-none"
    @click="globalClick"
    @contextmenu="closeCtxMenu"
  >
    <!-- Repo selector dropdown -->
    <div class="p-2.5 repo-dropdown relative">
      <button
        class="w-full flex items-center gap-2 px-2.5 py-2.5 rounded-[var(--radius)] text-xs bg-[--accent] text-white hover:bg-[--accent-hover] transition-colors truncate cursor-pointer"
        @click.stop="repoPath ? toggleDropdown() : selectOpenOther()"
      >
        <FolderOpen :size="14" class="flex-shrink-0" />
        <span class="truncate flex-1 text-left">{{ repoPath ? dirName(repoPath) : '打开本地仓库' }}</span>
        <ChevronDownIcon v-if="repoPath" :size="14" class="flex-shrink-0" />
      </button>

      <!-- Dropdown menu -->
      <div
        v-if="dropdownOpen"
        class="absolute left-2.5 right-2.5 top-full mt-1 bg-[--bg-tertiary] border border-[--border-color] rounded-[var(--radius)] shadow-md z-50 max-h-52 overflow-y-auto text-xs"
      >
        <div
          v-for="path in recentRepos"
          :key="path"
          class="flex items-center gap-2 px-2.5 py-2.5 cursor-pointer hover:bg-[--accent] hover:text-white transition-colors truncate"
          :class="path === repoPath ? 'text-[--accent] font-medium' : 'text-[--text-primary]'"
          @click.stop="selectPath(path)"
        >
          <FolderOpen :size="13" class="flex-shrink-0" />
          <span class="truncate">{{ dirName(path) }}</span>
        </div>

        <div
          v-if="recentRepos.length > 0"
          class="h-px bg-[--border-color] mx-2.5"
        />

        <div
          class="flex items-center gap-2 px-2.5 py-2.5 cursor-pointer text-[--text-secondary] hover:bg-[--accent] hover:text-white transition-colors"
          @click.stop="selectOpenOther"
        >
          <FolderOpen :size="13" class="flex-shrink-0" />
          <span>打开其他仓库...</span>
        </div>
      </div>

      <div
        v-if="repoPath"
        class="text-[10px] text-[--text-secondary] px-2.5 mt-1 truncate font-mono-ui"
        :title="repoPath"
      >
        {{ repoPath }}
      </div>
    </div>

    <!-- Tab Switcher -->
    <div class="flex border-b border-[--border-color]">
      <button
        class="flex-1 flex items-center justify-center gap-1.5 px-2.5 py-2 text-xs transition-colors cursor-pointer"
        :class="activeTab === 'workspace'
          ? 'text-[--text-primary] border-b-2 border-[--accent]'
          : 'text-[--text-secondary] hover:text-[--text-primary]'"
        @click="emit('switchTab', 'workspace')"
      >
        <List :size="12" />
        <span>工作区</span>
      </button>
      <button
        class="flex-1 flex items-center justify-center gap-1.5 px-2.5 py-2 text-xs transition-colors cursor-pointer"
        :class="activeTab === 'history'
          ? 'text-[--text-primary] border-b-2 border-[--accent]'
          : 'text-[--text-secondary] hover:text-[--text-primary]'"
        @click="emit('switchTab', 'history')"
      >
        <History :size="12" />
        <span>历史</span>
      </button>
    </div>

    <!-- Fetch / Push / Pull / Branch -->
    <div v-if="repoPath" class="flex flex-wrap gap-1 px-2.5 py-1 border-b border-[--border-color]">
      <button
        class="flex items-center gap-1.5 px-2 py-1 rounded-[var(--radius)] text-xs transition-colors whitespace-nowrap cursor-pointer"
        :class="fetchLoading
          ? 'text-[--accent] bg-[--bg-tertiary] cursor-wait'
          : 'text-[--text-secondary] hover:text-[--text-primary] hover:bg-[--bg-tertiary]'"
        :disabled="fetchLoading"
        @click="emit('fetch')"
      >
        <Loader2 v-if="fetchLoading" :size="12" class="animate-spin flex-shrink-0" />
        <RefreshCw v-else :size="12" class="flex-shrink-0" />
        <span>Fetch</span>
      </button>
      <button
        class="flex items-center gap-1.5 px-2 py-1 rounded-[var(--radius)] text-xs transition-colors relative whitespace-nowrap cursor-pointer"
        :class="pullLoading
          ? 'text-[--accent] bg-[--bg-tertiary] cursor-wait'
          : 'text-[--text-secondary] hover:text-[--text-primary] hover:bg-[--bg-tertiary]'"
        :disabled="pullLoading"
        @click="emit('pull')"
      >
        <Loader2 v-if="pullLoading" :size="12" class="animate-spin flex-shrink-0" />
        <ArrowDownToLine v-else :size="12" class="flex-shrink-0" />
        <span>Pull</span>
        <span
          v-if="!pullLoading && aheadBehind.behind > 0"
          class="absolute -top-0.5 -right-0.5 flex h-4 min-w-[1rem] items-center justify-center rounded-full px-1 text-[10px] font-semibold leading-none bg-[--accent] text-white"
        >{{ aheadBehind.behind }}</span>
      </button>
      <button
        class="flex items-center gap-1.5 px-2 py-1 rounded-[var(--radius)] text-xs transition-colors relative whitespace-nowrap cursor-pointer"
        :class="pushLoading
          ? 'text-[--accent] bg-[--bg-tertiary] cursor-wait'
          : 'text-[--text-secondary] hover:text-[--text-primary] hover:bg-[--bg-tertiary]'"
        :disabled="pushLoading"
        @click="emit('push')"
      >
        <Loader2 v-if="pushLoading" :size="12" class="animate-spin flex-shrink-0" />
        <ArrowUpToLine v-else :size="12" class="flex-shrink-0" />
        <span>Push</span>
        <span
          v-if="!pushLoading && aheadBehind.ahead > 0"
          class="absolute -top-0.5 -right-0.5 flex h-4 min-w-[1rem] items-center justify-center rounded-full px-1 text-[10px] font-semibold leading-none bg-[--accent] text-white"
        >{{ aheadBehind.ahead }}</span>
      </button>
      <button
        class="flex items-center gap-1.5 px-2 py-1 rounded-[var(--radius)] text-xs text-[--text-secondary] hover:text-[--text-primary] hover:bg-[--bg-tertiary] transition-colors whitespace-nowrap cursor-pointer"
        title="从当前分支新建分支"
        @click.stop="openCreateBranch"
      >
        <GitBranch :size="12" />
        <span>分支</span>
      </button>
    </div>

    <!-- Branch tree -->
    <div class="flex-1 overflow-y-auto px-2.5 pb-2.5 pt-1">

      <div v-if="branchesLoading" class="flex items-center gap-1.5 pl-2 py-1 text-xs text-[--text-secondary]">
        <Loader2 :size="12" class="animate-spin" />
        <span>加载中...</span>
      </div>

      <template v-else-if="branches.length > 0">
        <!-- Local branches tree -->
        <div
          v-for="node in localBranches"
          :key="node.key"
          class="flex items-center gap-0.5 rounded text-xs leading-snug py-1 pr-2"
          :class="node.isLeaf
            ? (node.isCurrent
              ? 'text-[--accent] font-medium cursor-default'
              : 'text-[--text-secondary] cursor-pointer hover:text-[--text-primary]')
            : 'text-[--text-secondary] hover:text-[--text-primary]'"
          :style="{ paddingLeft: (node.depth * 12 + 10) + 'px' }"
          @dblclick="node.isLeaf && !node.isCurrent && emit('checkoutBranch', node.label)"
          @contextmenu.prevent.stop="node.isLeaf && handleContextMenu($event, node.label, node.isCurrent)"
        >
          <button
            v-if="!node.isLeaf"
            class="flex-shrink-0 p-1 rounded hover:bg-[--bg-tertiary] cursor-pointer"
            @click.stop="toggleGroup(node.key)"
          >
            <ChevronRight
              v-if="!expandedGroups.has(node.key)"
              :size="12"
              class="text-[--text-secondary]"
            />
            <ChevronDown
              v-else
              :size="12"
              class="text-[--text-secondary]"
            />
          </button>

          <GitBranch v-if="node.isLeaf" :size="12" class="flex-shrink-0" />

          <span class="truncate">{{ node.label }}</span>
          <!-- <span
            v-if="node.isHead"
            class="flex-shrink-0 ml-1 inline-flex items-center rounded px-1.5 py-0.5 text-[10px] font-semibold bg-[--accent] text-white leading-none"
          >HEAD</span> -->
        </div>

        <!-- Remote branches -->
        <div
          v-if="remoteBranches.length > 0"
          class="flex items-center gap-1.5 mt-1 mb-0.5 pl-2 text-[10px] text-[--text-secondary] uppercase tracking-wide"
        >
          <Globe :size="10" />
          <span>远程</span>
        </div>
        <div
          v-for="node in remoteBranches"
          :key="node.key"
          class="flex items-center gap-0.5 rounded text-xs leading-snug py-1 pr-2"
          :class="node.isLeaf
            ? 'text-[--text-secondary] cursor-pointer hover:text-[--text-primary]'
            : 'text-[--text-secondary] hover:text-[--text-primary]'"
          :style="{ paddingLeft: (node.depth * 12 + 10) + 'px' }"
          @click="node.isLeaf && emit('checkoutRemote', node.key)"
        >
          <button
            v-if="!node.isLeaf"
            class="flex-shrink-0 p-1 rounded hover:bg-[--bg-tertiary] cursor-pointer"
            @click.stop="toggleGroup(node.key)"
          >
            <ChevronRight
              v-if="!expandedGroups.has(node.key)"
              :size="12"
              class="text-[--text-secondary]"
            />
            <ChevronDown
              v-else
              :size="12"
              class="text-[--text-secondary]"
            />
          </button>

          <Globe v-if="node.isLeaf" :size="12" class="flex-shrink-0" />

          <span class="truncate">{{ node.label }}</span>
          <span
            v-if="node.isHead"
            class="flex-shrink-0 ml-1 inline-flex items-center rounded px-1.5 py-0.5 text-[10px] font-semibold bg-[--accent] text-white leading-none"
          >HEAD</span>
        </div>
      </template>

      <div v-else class="text-xs text-[--text-secondary] pl-2 py-1">
        尚未打开仓库
      </div>
    </div>

    <!-- Context menu -->
    <Teleport to="body">
      <div
        v-if="ctxMenu"
        class="fixed z-[9999] bg-[--bg-tertiary] border border-[--border-color] rounded-[var(--radius)] shadow-lg p-2.5 text-xs min-w-[120px]"
        :style="{ left: ctxMenu.x + 'px', top: ctxMenu.y + 'px' }"
        @click.stop
      >
        <button
          class="w-full text-left px-2.5 py-2.5 rounded-[var(--radius)] text-[--text-primary] hover:bg-[--accent] hover:text-white transition-colors cursor-pointer"
          @click="ctxRename"
        >重命名</button>
        <template v-if="!ctxMenu.isCurrent">
          <div class="h-px bg-[--border-color] my-2" />
          <button
            class="w-full text-left px-2.5 py-2.5 rounded-[var(--radius)] text-[--text-primary] hover:bg-[--accent] hover:text-white transition-colors cursor-pointer"
            @click="ctxMerge"
          >合并到当前分支</button>
          <button
            class="w-full text-left px-2.5 py-2.5 rounded-[var(--radius)] text-red-400 hover:bg-red-700 hover:text-white transition-colors cursor-pointer"
            @click="ctxDelete"
          >删除</button>
        </template>
      </div>
    </Teleport>

    <!-- Create branch dialog -->
    <Teleport to="body">
      <div
        v-if="createBranchDialog"
        class="fixed inset-0 z-[9999] flex items-center justify-center bg-black/40"
        @click="cancelCreateBranch"
      >
        <div
          class="bg-[--bg-tertiary] border border-[--border-color] rounded-[var(--radius)] shadow-xl p-2.5 w-[280px]"
          @click.stop
        >
          <div class="text-xs text-[--text-primary] mb-2 font-medium">新建分支</div>
          <div class="text-[10px] text-[--text-secondary] mb-2">从当前分支创建新分支</div>
          <input
            ref="createBranchInput"
            v-model="newBranchName"
            placeholder="输入分支名称..."
            class="w-full px-2.5 py-2.5 rounded-[var(--radius)] bg-[--bg-secondary] border border-[--border-color] text-xs text-[--text-primary] outline-none focus:border-[--accent] transition-colors font-mono-ui"
            @keydown.enter="submitCreateBranch"
            @keydown.escape="cancelCreateBranch"
          />
          <div class="flex justify-end gap-2 mt-2.5">
            <button
              class="px-2.5 py-2.5 rounded-[var(--radius)] text-xs text-[--text-secondary] hover:text-[--text-primary] hover:bg-[--bg-secondary] transition-colors cursor-pointer"
              @click="cancelCreateBranch"
            >取消</button>
            <button
              class="px-2.5 py-2.5 rounded-[var(--radius)] text-xs bg-[--accent] text-white hover:bg-[--accent-hover] transition-colors cursor-pointer"
              @click="submitCreateBranch"
            >创建</button>
          </div>
        </div>
      </div>
    </Teleport>

    <!-- Rename dialog -->
    <Teleport to="body">
      <div
        v-if="renameDialog"
        class="fixed inset-0 z-[9999] flex items-center justify-center bg-black/40"
        @click="cancelRename"
      >
        <div
          class="bg-[--bg-tertiary] border border-[--border-color] rounded-[var(--radius)] shadow-xl p-2.5 w-[280px]"
          @click.stop
        >
          <div class="text-xs text-[--text-primary] mb-2 font-medium">重命名分支</div>
          <div class="text-[10px] text-[--text-secondary] mb-2 font-mono-ui">{{ renameDialog.branch }}</div>
          <input
            ref="renameInput"
            v-model="renameDialog.value"
            class="w-full px-2.5 py-2.5 rounded-[var(--radius)] bg-[--bg-secondary] border border-[--border-color] text-xs text-[--text-primary] outline-none focus:border-[--accent] transition-colors font-mono-ui"
            @keydown.enter="submitRename"
            @keydown.escape="cancelRename"
          />
          <div class="flex justify-end gap-2 mt-2.5">
            <button
              class="px-2.5 py-2.5 rounded-[var(--radius)] text-xs text-[--text-secondary] hover:text-[--text-primary] hover:bg-[--bg-secondary] transition-colors cursor-pointer"
              @click="cancelRename"
            >取消</button>
            <button
              class="px-2.5 py-2.5 rounded-[var(--radius)] text-xs bg-[--accent] text-white hover:bg-[--accent-hover] transition-colors cursor-pointer"
              @click="submitRename"
            >确定</button>
          </div>
        </div>
      </div>
    </Teleport>
  </div>
</template>
