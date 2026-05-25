<script setup lang="ts">
import { computed } from 'vue'
import { User, CalendarDays, Hash } from 'lucide-vue-next'
import type { CommitLog } from '../types'
import {
  layoutCommitGraph,
  shortHash,
  formatCommitDate,
  GRAPH_ROW_HEIGHT,
  GRAPH_LANE_WIDTH,
  GRAPH_NODE_RADIUS,
} from '../utils/commitGraph'

const props = defineProps<{
  logs: CommitLog[]
  selectedHash: string | null
}>()

const emit = defineEmits<{
  selectCommit: [hash: string]
}>()

const layout = computed(() => layoutCommitGraph(props.logs))

const graphWidth = computed(() => layout.value.laneCount * GRAPH_LANE_WIDTH + 8)

</script>

<template>
  <div class="relative" :style="{ height: `${layout.height}px` }">
    <svg
      class="absolute top-0 left-0 pointer-events-none z-[1]"
      :width="graphWidth"
      :height="layout.height"
    >
      <path
        v-for="(segment, index) in layout.paths"
        :key="`path-${index}`"
        :d="segment.d"
        fill="none"
        :stroke="segment.color"
        stroke-width="2"
        stroke-linecap="round"
        stroke-linejoin="round"
        opacity="0.85"
      />
      <circle
        v-for="node in layout.nodes"
        :key="`node-${node.hash}`"
        :cx="node.column * GRAPH_LANE_WIDTH + GRAPH_LANE_WIDTH / 2"
        :cy="node.row * GRAPH_ROW_HEIGHT + GRAPH_ROW_HEIGHT / 2"
        :r="GRAPH_NODE_RADIUS"
        :fill="node.color"
        :stroke="node.hash === selectedHash ? 'var(--text-primary)' : 'var(--bg-secondary)'"
        :stroke-width="node.hash === selectedHash ? 2 : 1"
      />
    </svg>

    <div
      v-for="(log, row) in logs"
      :key="log.hash"
      class="absolute left-0 right-0 flex items-stretch border-b border-[--border-color] cursor-pointer"
      :style="{ top: `${row * GRAPH_ROW_HEIGHT}px`, height: `${GRAPH_ROW_HEIGHT}px` }"
      @click="emit('selectCommit', log.hash)"
    >
      <div class="shrink-0 pointer-events-none" :style="{ width: `${graphWidth}px` }" />
      <div
        class="flex-1 min-w-0 px-2.5 py-2 flex flex-col justify-center border-l border-[--border-color] transition-colors"
        :class="log.hash === selectedHash
          ? 'bg-[--bg-tertiary] border-l-2 border-l-[--accent]'
          : 'hover:bg-[--bg-tertiary]'"
      >
        <div class="flex items-center gap-1.5 min-w-0">
          <span class="text-xs text-[--text-primary] font-medium truncate">{{ log.message }}</span>
          <span
            v-for="ref in log.refs"
            :key="ref"
            class="shrink-0 px-1.5 py-0.5 rounded text-[9px] font-mono-ui bg-[--accent]/15 text-[--accent]"
          >{{ ref }}</span>
        </div>
        <div class="flex items-center flex-wrap gap-x-2 gap-y-0.5 mt-0.5 text-[10px] text-[--text-secondary] font-mono-ui">
          <span class="flex items-center gap-1">
            <Hash :size="10" />
            {{ shortHash(log.hash) }}
          </span>
          <span class="flex items-center gap-1 truncate">
            <User :size="10" />
            {{ log.author }}
          </span>
          <span class="flex items-center gap-1">
            <CalendarDays :size="10" />
            {{ formatCommitDate(log.date) }}
          </span>
        </div>
      </div>
    </div>
  </div>
</template>
