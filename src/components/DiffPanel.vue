<script setup lang="ts">
import { computed } from 'vue'

const props = defineProps<{
  diffText: string
  fileName: string | null
}>()

interface DiffLine {
  type: 'header' | 'added' | 'removed' | 'context'
  content: string
}

const diffLines = computed((): DiffLine[] => {
  if (!props.diffText) return []
  return props.diffText.split('\n').map((line) => {
    if (line.startsWith('+')) {
      return { type: 'added', content: line }
    }
    if (line.startsWith('-')) {
      return { type: 'removed', content: line }
    }
    if (line.startsWith('@@')) {
      return { type: 'header', content: line }
    }
    return { type: 'context', content: line }
  })
})
</script>

<template>
  <div class="h-full flex flex-col bg-[--bg-primary]">
    <!-- Diff header -->
    <div class="px-4 py-2 text-xs text-[--text-secondary] border-b border-[--border-color] flex items-center gap-2">
      <span v-if="fileName" class="text-[--text-primary] font-medium truncate">{{ fileName }}</span>
      <span v-else class="text-[--text-secondary]">选择文件查看差异</span>
    </div>

    <!-- Diff content -->
    <div class="flex-1 overflow-y-auto font-mono text-xs leading-relaxed">
      <div v-if="!diffText" class="p-4 text-[--text-secondary] text-sm">
        <template v-if="fileName">没有差异内容</template>
        <template v-else>点击左侧文件查看变更</template>
      </div>
      <div v-else class="py-1">
        <div
          v-for="(line, i) in diffLines"
          :key="i"
          class="px-4 whitespace-pre-wrap"
          :class="{
            'bg-[--diff-added] text-[--diff-added-text]': line.type === 'added',
            'bg-[--diff-removed] text-[--diff-removed-text]': line.type === 'removed',
            'text-[--text-secondary]': line.type === 'header',
            'text-[--text-primary]': line.type === 'context',
          }"
        >{{ line.content }}</div>
      </div>
    </div>
  </div>
</template>
