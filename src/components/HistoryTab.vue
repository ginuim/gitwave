<script setup lang="ts">
import { ref, watch } from 'vue'
import { User, CalendarDays, Hash, Loader2, List, GitBranch, GitMerge } from 'lucide-vue-next'
import type { CommitLog } from '../types'
import CommitGraphView from './CommitGraphView.vue'
import {
  shortHash,
  formatCommitDate,
  isMergeCommit,
  HISTORY_VIEW_STORAGE_KEY,
} from '../utils/commitGraph'

defineProps<{
  logs: CommitLog[]
  loading: boolean
  loadingMore: boolean
  hasMore: boolean
  selectedHash: string | null
  filter: 'current' | 'all'
  currentBranch: string
}>()

const emit = defineEmits<{
  selectCommit: [hash: string]
  updateFilter: [filter: 'current' | 'all']
  loadMore: []
}>()

function readStoredViewMode(): 'list' | 'graph' {
  try {
    const stored = localStorage.getItem(HISTORY_VIEW_STORAGE_KEY)
    return stored === 'graph' ? 'graph' : 'list'
  } catch {
    return 'list'
  }
}

const viewMode = ref<'list' | 'graph'>(readStoredViewMode())
const scrollRoot = ref<HTMLElement | null>(null)

watch(viewMode, (mode) => {
  try {
    localStorage.setItem(HISTORY_VIEW_STORAGE_KEY, mode)
  } catch {
    // ignore storage errors
  }
})

function onScroll(event: Event) {
  const target = event.target as HTMLElement
  if (target.scrollTop + target.clientHeight >= target.scrollHeight - 120) {
    emit('loadMore')
  }
}
</script>

<template>
  <div class="h-full flex flex-col bg-[--bg-secondary] min-w-[320px]">
    <div class="flex items-center justify-between px-2.5 py-2.5 bg-[--bg-tertiary] border-b border-[--border-color] text-xs gap-2">
      <span class="text-xs text-[--text-secondary] uppercase tracking-wide font-mono-ui shrink-0">
        Commits ({{ logs.length }}<template v-if="hasMore">+</template>)
      </span>
      <div class="flex items-center gap-2 min-w-0">
        <div class="flex items-center rounded-[var(--radius)] border border-[--border-color] overflow-hidden shrink-0">
          <button
            class="inline-flex items-center gap-1 px-2 py-1 text-xs transition-colors cursor-pointer"
            :class="viewMode === 'list'
              ? 'bg-[--accent] text-white'
              : 'text-[--text-secondary] hover:text-[--text-primary]'"
            @click="viewMode = 'list'"
          >
            <List :size="12" />
            列表
          </button>
          <button
            class="inline-flex items-center gap-1 px-2 py-1 text-xs transition-colors cursor-pointer border-l border-[--border-color]"
            :class="viewMode === 'graph'
              ? 'bg-[--accent] text-white'
              : 'text-[--text-secondary] hover:text-[--text-primary]'"
            @click="viewMode = 'graph'"
          >
            <GitBranch :size="12" />
            分支图
          </button>
        </div>
        <button
          class="px-2 py-1 rounded-[var(--radius)] text-xs transition-colors cursor-pointer leading-tight max-w-[120px] truncate"
          :class="filter === 'current'
            ? 'bg-[--accent] text-white font-medium'
            : 'text-[--text-secondary] hover:text-[--text-primary]'"
          :title="currentBranch || '当前'"
          @click="emit('updateFilter', 'current')"
        >{{ currentBranch || '当前' }}</button>
        <button
          class="px-2 py-1 rounded-[var(--radius)] text-xs transition-colors cursor-pointer leading-tight shrink-0"
          :class="filter === 'all'
            ? 'bg-[--accent] text-white font-medium'
            : 'text-[--text-secondary] hover:text-[--text-primary]'"
          @click="emit('updateFilter', 'all')"
        >全部</button>
      </div>
    </div>

    <div
      v-if="viewMode === 'graph' && logs.length > 0"
      class="px-2.5 py-1.5 text-[10px] leading-relaxed text-[--text-secondary] border-b border-[--border-color] bg-[--bg-secondary]"
    >
      竖线表示提交链，线条汇合为合并。悬停可高亮路径；多泳道时可在右上角筛选。
    </div>

    <div v-if="loading && logs.length === 0" class="flex items-center justify-center py-2.5 text-[--text-secondary]">
      <Loader2 :size="14" class="animate-spin mr-2.5" />
      <span class="text-xs">加载中...</span>
    </div>
    <div v-else-if="logs.length === 0" class="px-2.5 py-2.5 text-xs text-[--text-secondary] text-center">
      没有提交记录
    </div>
    <div
      v-else
      ref="scrollRoot"
      class="flex-1 overflow-y-auto overflow-x-auto"
      @scroll="onScroll"
    >
      <template v-if="viewMode === 'list'">
        <div
          v-for="log in logs"
          :key="log.hash"
          class="px-2.5 py-2.5 border-b border-[--border-color] cursor-pointer transition-colors"
          :class="log.hash === selectedHash ? 'bg-[--bg-tertiary] border-l-2 border-l-[--accent]' : 'hover:bg-[--bg-tertiary]'"
          @click="emit('selectCommit', log.hash)"
        >
          <div class="flex items-center gap-1.5 min-w-0">
            <span class="text-xs text-[--text-primary] font-medium leading-relaxed truncate">{{ log.message }}</span>
            <span
              v-if="isMergeCommit(log)"
              class="shrink-0 inline-flex items-center gap-0.5 px-1.5 py-0.5 rounded text-[9px] font-mono-ui bg-amber-500/15 text-amber-600 dark:text-amber-400"
            >
              <GitMerge :size="9" />
              合并
            </span>
            <span
              v-for="ref in log.refs"
              :key="ref"
              class="shrink-0 px-1.5 py-0.5 rounded text-[9px] font-mono-ui bg-[--accent]/15 text-[--accent]"
            >{{ ref }}</span>
          </div>
          <div class="flex items-center flex-wrap gap-x-2 gap-y-1 mt-1 text-[10px] text-[--text-secondary] font-mono-ui">
            <span class="flex items-center gap-1">
              <Hash :size="10" />
              {{ shortHash(log.hash) }}
            </span>
            <span class="flex items-center gap-1">
              <User :size="10" />
              {{ log.author }}
            </span>
            <span class="flex items-center gap-1" :title="log.date">
              <CalendarDays :size="10" />
              {{ formatCommitDate(log.date) }}
            </span>
          </div>
        </div>
      </template>

      <CommitGraphView
        v-else
        :logs="logs"
        :selected-hash="selectedHash"
        @select-commit="emit('selectCommit', $event)"
      />

      <div v-if="loadingMore" class="flex items-center justify-center py-3 text-[--text-secondary]">
        <Loader2 :size="14" class="animate-spin mr-2" />
        <span class="text-xs">加载更多...</span>
      </div>
      <div v-else-if="!hasMore && logs.length > 0" class="py-3 text-center text-[10px] text-[--text-secondary]">
        已加载全部
      </div>
    </div>
  </div>
</template>
