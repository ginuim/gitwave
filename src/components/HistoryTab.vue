<script setup lang="ts">
import { User, CalendarDays, Hash, Loader2 } from 'lucide-vue-next'
import type { CommitLog } from '../types'

defineProps<{
  logs: CommitLog[]
  loading: boolean
  selectedHash: string | null
  filter: 'current' | 'all'
  currentBranch: string
}>()

const emit = defineEmits<{
  selectCommit: [hash: string]
  updateFilter: [filter: 'current' | 'all']
}>()
</script>

<template>
  <div class="h-full flex flex-col bg-[--bg-secondary]">
    <!-- Header with filter toggle -->
    <div class="flex items-center justify-between px-3 py-1.5 bg-[--bg-tertiary] border-b border-[--border-color] text-xs">
      <span class="text-[--text-secondary] uppercase tracking-wider">
        Commits ({{ logs.length }})
      </span>
      <div class="flex items-center gap-1">
        <button
          class="px-2 py-0.5 rounded text-[11px] transition-colors"
          :class="filter === 'current'
            ? 'bg-[--accent] text-white font-medium'
            : 'text-[--text-secondary] hover:text-[--text-primary]'"
          @click="emit('updateFilter', 'current')"
        >{{ currentBranch || '当前' }}</button>
        <button
          class="px-2 py-0.5 rounded text-[11px] transition-colors"
          :class="filter === 'all'
            ? 'bg-[--accent] text-white font-medium'
            : 'text-[--text-secondary] hover:text-[--text-primary]'"
          @click="emit('updateFilter', 'all')"
        >全部</button>
      </div>
    </div>

    <div v-if="loading" class="flex items-center justify-center py-12 text-[--text-secondary]">
      <Loader2 :size="16" class="animate-spin mr-2" />
      <span class="text-xs">加载中...</span>
    </div>
    <div v-else-if="logs.length === 0" class="px-4 py-6 text-xs text-[--text-secondary] text-center">
      没有提交记录
    </div>
    <div v-else class="flex-1 overflow-y-auto">
      <div
        v-for="log in logs"
        :key="log.hash"
        class="px-3 py-2.5 border-b border-[--border-color] cursor-pointer transition-colors"
        :class="log.hash === selectedHash ? 'bg-[--bg-tertiary] border-l-2 border-l-[--accent]' : 'hover:bg-[--bg-tertiary]'"
        @click="emit('selectCommit', log.hash)"
      >
        <div class="text-xs text-[--text-primary] font-medium leading-relaxed">
          {{ log.message }}
        </div>
        <div class="flex items-center gap-3 mt-1 text-[10px] text-[--text-secondary]">
          <span class="flex items-center gap-1">
            <Hash :size="10" />
            {{ log.hash }}
          </span>
          <span class="flex items-center gap-1">
            <User :size="10" />
            {{ log.author }}
          </span>
          <span class="flex items-center gap-1">
            <CalendarDays :size="10" />
            {{ log.date }}
          </span>
        </div>
      </div>
    </div>
  </div>
</template>
