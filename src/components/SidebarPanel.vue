<script setup lang="ts">
import { ref, computed } from 'vue'
import {
  FolderOpen, GitBranch, Globe, ArrowUp, ArrowDown,
  List, History, Loader2, ChevronRight, ChevronDown,
} from 'lucide-vue-next'
import type { BranchInfo } from '../types'

const props = defineProps<{
  repoPath: string | null
  activeTab: 'workspace' | 'history'
  pushPullLoading: boolean
  branches: BranchInfo[]
  branchesLoading: boolean
}>()

const emit = defineEmits<{
  openRepo: []
  switchTab: [tab: 'workspace' | 'history']
  push: []
  pull: []
}>()

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
  // trigger reactivity
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
        // leaf
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
        // Check if group has any leaf descendants
        const hasLeaves = hasLeafDescendant(val)
        result.push({
          label: key,
          depth,
          isLeaf: false,
          isCurrent: false,
          isRemote: false,
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
  <div class="h-full flex flex-col bg-[--bg-secondary] border-r border-[--border-color] select-none">
    <!-- Header / Open Repo -->
    <div class="p-3 flex flex-col gap-2">
      <button
        class="flex items-center gap-2 px-3 py-2 rounded text-sm bg-[--accent] text-white hover:bg-[--accent-hover] transition-colors"
        @click="emit('openRepo')"
      >
        <FolderOpen :size="16" />
        <span>打开本地仓库</span>
      </button>

      <div
        v-if="repoPath"
        class="text-xs text-[--text-secondary] px-1 truncate"
        :title="repoPath"
      >
        {{ repoPath }}
      </div>
    </div>

    <!-- Tab Switcher -->
    <div class="flex border-b border-[--border-color]">
      <button
        class="flex-1 flex items-center justify-center gap-1.5 px-3 py-2 text-xs transition-colors"
        :class="activeTab === 'workspace'
          ? 'text-[--text-primary] border-b-2 border-[--accent]'
          : 'text-[--text-secondary] hover:text-[--text-primary]'"
        @click="emit('switchTab', 'workspace')"
      >
        <List :size="14" />
        <span>工作区</span>
      </button>
      <button
        class="flex-1 flex items-center justify-center gap-1.5 px-3 py-2 text-xs transition-colors"
        :class="activeTab === 'history'
          ? 'text-[--text-primary] border-b-2 border-[--accent]'
          : 'text-[--text-secondary] hover:text-[--text-primary]'"
        @click="emit('switchTab', 'history')"
      >
        <History :size="14" />
        <span>历史</span>
      </button>
    </div>

    <!-- Push / Pull -->
    <div v-if="repoPath" class="flex gap-1 px-3 py-2 border-b border-[--border-color]">
      <button
        class="flex items-center gap-1 px-2 py-1 rounded text-xs text-[--text-secondary] hover:text-[--text-primary] hover:bg-[--bg-tertiary] transition-colors disabled:opacity-40"
        :disabled="pushPullLoading"
        @click="emit('push')"
      >
        <ArrowUp :size="14" />
        <span>Push</span>
      </button>
      <button
        class="flex items-center gap-1 px-2 py-1 rounded text-xs text-[--text-secondary] hover:text-[--text-primary] hover:bg-[--bg-tertiary] transition-colors disabled:opacity-40"
        :disabled="pushPullLoading"
        @click="emit('pull')"
      >
        <ArrowDown :size="14" />
        <span>Pull</span>
      </button>
    </div>

    <!-- Branch tree -->
    <div class="flex-1 overflow-y-auto p-3 pt-1">

      <div v-if="branchesLoading" class="flex items-center gap-2 pl-1 py-2 text-xs text-[--text-secondary]">
        <Loader2 :size="12" class="animate-spin" />
        <span>加载中...</span>
      </div>

      <template v-else-if="branches.length > 0">
        <!-- Local branches tree -->
        <div
          v-for="node in localBranches"
          :key="node.key"
          class="flex items-center gap-1 rounded text-xs cursor-default"
          :class="node.isLeaf
            ? (node.isCurrent ? 'text-[--accent] font-medium' : 'text-[--text-secondary] hover:text-[--text-primary]')
            : 'text-[--text-secondary] hover:text-[--text-primary]'"
          :style="{ paddingLeft: (node.depth * 14 + 4) + 'px' }"
        >
          <!-- Group toggle -->
          <button
            v-if="!node.isLeaf"
            class="flex-shrink-0 p-0.5 rounded hover:bg-[--bg-tertiary]"
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

          <!-- Leaf spacer for alignment -->
          <div v-if="node.isLeaf" class="w-5 flex-shrink-0" />

          <!-- Icon -->
          <GitBranch v-if="node.isLeaf" :size="11" class="flex-shrink-0" />
          <!-- Folder icon only renders as empty space to align - group icon is the chevron above -->

          <span class="truncate">{{ node.label }}</span>
          <span
            v-if="node.isHead"
            class="flex-shrink-0 ml-1 px-1 rounded text-[9px] font-semibold bg-[--accent] text-white leading-tight"
          >HEAD</span>
        </div>

        <!-- Remote branches -->
        <div
          v-if="remoteBranches.length > 0"
          class="flex items-center gap-1.5 mt-3 mb-1 pl-1 text-[10px] text-[--text-secondary] uppercase tracking-wider"
        >
          <Globe :size="10" />
          <span>远程</span>
        </div>
        <div
          v-for="node in remoteBranches"
          :key="node.key"
          class="flex items-center gap-1 rounded text-xs cursor-default"
          :class="node.isLeaf
            ? 'text-[--text-secondary] hover:text-[--text-primary]'
            : 'text-[--text-secondary] hover:text-[--text-primary]'"
          :style="{ paddingLeft: (node.depth * 14 + 4) + 'px' }"
        >
          <!-- Group toggle -->
          <button
            v-if="!node.isLeaf"
            class="flex-shrink-0 p-0.5 rounded hover:bg-[--bg-tertiary]"
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

          <div v-if="node.isLeaf" class="w-5 flex-shrink-0" />

          <Globe v-if="node.isLeaf" :size="11" class="flex-shrink-0" />

          <span class="truncate">{{ node.label }}</span>
          <span
            v-if="node.isHead"
            class="flex-shrink-0 ml-1 px-1 rounded text-[9px] font-semibold bg-[--accent] text-white leading-tight"
          >HEAD</span>
        </div>
      </template>

      <div v-else class="text-xs text-[--text-secondary] pl-1">
        尚未打开仓库
      </div>
    </div>
  </div>
</template>
