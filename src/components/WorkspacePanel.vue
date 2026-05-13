<script setup lang="ts">
import { ref } from 'vue'
import { FilePlus, FileMinus, GitCommitVertical, FileCode, Loader2 } from 'lucide-vue-next'
import type { FileStatus } from '../types'

defineProps<{
  statuses: FileStatus[]
  selectedFile: string | null
  commitLoading: boolean
  statusLoading: boolean
}>()

const emit = defineEmits<{
  stageFile: [path: string]
  unstageFile: [path: string]
  selectFile: [path: string, isStaged: boolean]
  commit: [message: string]
}>()

const commitMessage = ref('')

function handleCommit() {
  if (!commitMessage.value.trim()) return
  emit('commit', commitMessage.value)
  commitMessage.value = ''
}

const unstagedFiles = (statuses: FileStatus[]) => statuses.filter((f) => !f.isStaged)
const stagedFiles = (statuses: FileStatus[]) => statuses.filter((f) => f.isStaged)
</script>

<template>
  <div class="h-full flex flex-col bg-[--bg-secondary]">
    <!-- Unstaged Changes -->
    <div class="flex-1 overflow-y-auto min-h-0">
      <div class="px-3 py-1.5 text-xs text-[--text-secondary] uppercase tracking-wider bg-[--bg-tertiary] border-b border-[--border-color] sticky top-0 z-10">
        Unstaged Changes ({{ unstagedFiles(statuses).length }})
      </div>
      <div v-if="statusLoading" class="flex items-center justify-center py-8 text-[--text-secondary]">
        <Loader2 :size="16" class="animate-spin mr-2" />
        <span class="text-xs">加载中...</span>
      </div>
      <div v-else-if="unstagedFiles(statuses).length === 0" class="px-4 py-3 text-xs text-[--text-secondary]">
        没有未暂存的变更
      </div>
      <div v-else>
        <div
          v-for="file in unstagedFiles(statuses)"
          :key="file.path"
          class="flex items-center gap-1 px-3 py-1.5 text-xs border-b border-[--border-color] cursor-pointer hover:bg-[--bg-tertiary] transition-colors"
          :class="{ 'bg-[--bg-tertiary]': selectedFile === file.path }"
          @click="emit('selectFile', file.path, file.isStaged)"
        >
          <button
            class="flex-shrink-0 p-0.5 rounded text-[--diff-added-text] hover:text-green-300 transition-colors"
            title="暂存"
            @click.stop="emit('stageFile', file.path)"
          >
            <FilePlus :size="14" />
          </button>
          <FileCode :size="13" class="flex-shrink-0 text-[--text-secondary]" />
          <span class="truncate flex-1">{{ file.path }}</span>
          <span class="flex-shrink-0 text-[--diff-removed-text] font-mono">{{ file.status }}</span>
        </div>
      </div>

      <!-- Staged Changes -->
      <div class="px-3 py-1.5 text-xs text-[--text-secondary] uppercase tracking-wider bg-[--bg-tertiary] border-b border-[--border-color] sticky top-0 z-10">
        Staged Changes ({{ stagedFiles(statuses).length }})
      </div>
      <div v-if="stagedFiles(statuses).length === 0" class="px-4 py-3 text-xs text-[--text-secondary]">
        没有已暂存的变更
      </div>
      <div v-else>
        <div
          v-for="file in stagedFiles(statuses)"
          :key="file.path"
          class="flex items-center gap-1 px-3 py-1.5 text-xs border-b border-[--border-color] cursor-pointer hover:bg-[--bg-tertiary] transition-colors"
          :class="{ 'bg-[--bg-tertiary]': selectedFile === file.path }"
          @click="emit('selectFile', file.path, file.isStaged)"
        >
          <button
            class="flex-shrink-0 p-0.5 rounded text-[--diff-removed-text] hover:text-red-300 transition-colors"
            title="取消暂存"
            @click.stop="emit('unstageFile', file.path)"
          >
            <FileMinus :size="14" />
          </button>
          <FileCode :size="13" class="flex-shrink-0 text-[--text-secondary]" />
          <span class="truncate flex-1">{{ file.path }}</span>
          <span class="flex-shrink-0 text-[--diff-added-text] font-mono">{{ file.status }}</span>
        </div>
      </div>
    </div>

    <!-- Commit Form -->
    <div class="border-t border-[--border-color] p-3 bg-[--bg-secondary] flex-shrink-0">
      <textarea
        v-model="commitMessage"
        class="w-full px-3 py-2 rounded bg-[--bg-tertiary] border border-[--border-color] text-xs text-[--text-primary] placeholder-[--text-secondary] resize-none outline-none focus:border-[--accent] transition-colors"
        rows="2"
        placeholder="提交信息..."
        @keydown.meta.enter="handleCommit"
        @keydown.ctrl.enter="handleCommit"
      />
      <button
        class="mt-2 w-full flex items-center justify-center gap-1.5 px-3 py-1.5 rounded text-xs bg-[--accent] text-white hover:bg-[--accent-hover] transition-colors disabled:opacity-40 disabled:cursor-not-allowed"
        :disabled="!commitMessage.trim() || commitLoading"
        @click="handleCommit"
      >
        <GitCommitVertical :size="14" />
        <span>{{ commitLoading ? '提交中...' : 'Commit' }}</span>
      </button>
    </div>
  </div>
</template>
