<script setup lang="ts">
import { User, CalendarDays, Hash, Loader2 } from 'lucide-vue-next'
import type { CommitLog } from '../types'

defineProps<{
  logs: CommitLog[]
  loading: boolean
  selectedHash: string | null
}>()

const emit = defineEmits<{
  selectCommit: [hash: string]
}>()
</script>

<template>
  <div class="h-full flex flex-col bg-[--bg-secondary]">
    <div class="px-3 py-1.5 text-xs text-[--text-secondary] uppercase tracking-wider bg-[--bg-tertiary] border-b border-[--border-color]">
      Commits ({{ logs.length }})
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
