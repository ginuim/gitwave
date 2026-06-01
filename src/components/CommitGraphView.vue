<script setup lang="ts">
import { computed, ref, watch } from 'vue'
import { User, CalendarDays, Hash, GitMerge } from 'lucide-vue-next'
import type { BranchTip, CommitAction, CommitLog } from '../types'
import {
  collectRelatedHashes,
  isPathHighlighted,
  isMergeCommit,
  shortHash,
  formatCommitDate,
  laneForColumn,
  GRAPH_ROW_HEIGHT,
  GRAPH_LANE_WIDTH,
  GRAPH_NODE_RADIUS,
  GRAPH_NODE_RADIUS_ACTIVE,
  type GraphPath,
  type CommitGraphLayout,
  type GraphLane,
} from '../utils/commitGraph'

const props = defineProps<{
  logs: CommitLog[]
  branchTips: BranchTip[]
  selectedHash: string | null
  graphLayout: CommitGraphLayout
  displayLanes: GraphLane[]
}>()

const emit = defineEmits<{
  selectCommit: [hash: string]
  commitHover: [payload: { hash: string; commits: CommitLog[] }]
  commitAction: [payload: { action: CommitAction; hash: string }]
  branchTipClick: [ref: string]
}>()

const hoveredHash = ref<string | null>(null)
const hoveredLaneColumn = ref<number | null>(null)
const contextMenu = ref<{ x: number; y: number; hash: string } | null>(null)

const layout = computed(() => props.graphLayout)
const laneCount = computed(() => Math.max(props.displayLanes.length, 1))
const graphWidth = computed(() => laneCount.value * GRAPH_LANE_WIDTH)
const branchTipNames = computed(() => new Set(props.branchTips.map((tip) => tip.name)))

const focusHash = computed(() => hoveredHash.value ?? props.selectedHash)
const focusSet = computed(() => collectRelatedHashes(focusHash.value, props.logs))
const hasFocus = computed(() => focusSet.value !== null)

const hoveredLaneLabel = computed(() => {
  if (hoveredLaneColumn.value === null) return null
  return laneForColumn(props.displayLanes, hoveredLaneColumn.value)?.label
    ?? (hoveredLaneColumn.value === 0 ? '主线' : `分支 ${hoveredLaneColumn.value}`)
})

watch(() => props.logs, (commits) => {
  if (hoveredHash.value && !commits.some((commit) => commit.hash === hoveredHash.value)) {
    hoveredHash.value = null
  }
})

function nodeRadius(hash: string): number {
  if (hash === focusHash.value) return GRAPH_NODE_RADIUS_ACTIVE
  return GRAPH_NODE_RADIUS
}

function nodeOpacity(hash: string): number {
  if (!hasFocus.value) return 1
  return focusSet.value!.has(hash) ? 1 : 0.35
}

function pathOpacity(path: GraphPath): number {
  if (!hasFocus.value) return 0.55
  return isPathHighlighted(path, focusSet.value) ? 0.95 : 0.1
}

function pathWidth(path: GraphPath): number {
  if (!hasFocus.value) return 1.5
  return isPathHighlighted(path, focusSet.value) ? 2.5 : 1.5
}

function onRowEnter(hash: string) {
  hoveredHash.value = hash
  emit('commitHover', { hash, commits: props.logs })
}

function clearHover() {
  hoveredHash.value = null
  hoveredLaneColumn.value = null
}

function onLaneEnter(column: number) {
  hoveredLaneColumn.value = column
}

function onLaneLeave() {
  hoveredLaneColumn.value = null
}

function onGraphClick(event: MouseEvent) {
  const row = Math.floor(event.offsetY / GRAPH_ROW_HEIGHT)
  const log = props.logs[row]
  if (log) emit('selectCommit', log.hash)
}

function onGraphMouseMove(event: MouseEvent) {
  const row = Math.floor(event.offsetY / GRAPH_ROW_HEIGHT)
  const log = props.logs[row]
  if (log && log.hash !== hoveredHash.value) {
    onRowEnter(log.hash)
  }
}

function openCommitMenu(event: MouseEvent, hash: string) {
  event.preventDefault()
  contextMenu.value = { x: event.clientX, y: event.clientY, hash }
}

function runCommitAction(action: CommitAction) {
  const hash = contextMenu.value?.hash
  contextMenu.value = null
  if (!hash) return
  emit('commitAction', { action, hash })
}

function onRefClick(ref: string) {
  if (!branchTipNames.value.has(ref)) return
  emit('branchTipClick', ref)
}

defineExpose({ clearHover })
</script>

<template>
  <div
    v-if="logs.length === 0"
    class="px-2.5 py-6 text-xs text-[--text-secondary] text-center"
  >
    当前泳道筛选下没有提交，请勾选更多泳道
  </div>

  <div v-else class="relative" :style="{ height: `${layout.height}px` }" @mouseleave="clearHover">
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
        :stroke-width="pathWidth(segment)"
        stroke-linecap="round"
        stroke-linejoin="round"
        :opacity="pathOpacity(segment)"
        class="transition-opacity duration-150"
      />
      <circle
        v-for="node in layout.nodes"
        :key="`node-${node.hash}`"
        :cx="node.column * GRAPH_LANE_WIDTH + GRAPH_LANE_WIDTH / 2"
        :cy="node.row * GRAPH_ROW_HEIGHT + GRAPH_ROW_HEIGHT / 2"
        :r="nodeRadius(node.hash)"
        :fill="node.color"
        :stroke="node.hash === focusHash ? '#ffffff' : 'var(--bg-secondary)'"
        :stroke-width="node.hash === focusHash ? 2 : 1"
        :opacity="nodeOpacity(node.hash)"
        class="transition-all duration-150"
      />
    </svg>

    <div
      v-for="(log, row) in logs"
      :key="log.hash"
      class="absolute left-0 right-0 z-[2] flex items-stretch border-b border-[--border-color] cursor-pointer"
      :style="{ top: `${row * GRAPH_ROW_HEIGHT}px`, height: `${GRAPH_ROW_HEIGHT}px` }"
      @mouseenter="onRowEnter(log.hash)"
      @click="emit('selectCommit', log.hash)"
      @contextmenu="openCommitMenu($event, log.hash)"
    >
      <div class="shrink-0 pointer-events-none" :style="{ width: `${graphWidth}px` }" />
      <div
        class="flex-1 min-w-0 px-2.5 py-2 flex flex-col justify-center border-l border-[--border-color] transition-colors"
        :class="log.hash === selectedHash
          ? 'bg-[--bg-tertiary] border-l-2 border-l-[--accent]'
          : log.hash === hoveredHash
            ? 'bg-[--bg-tertiary]/70'
            : 'hover:bg-[--bg-tertiary]/70'"
      >
        <div class="flex items-center gap-1.5 min-w-0">
          <span class="text-xs text-[--text-primary] font-medium truncate">{{ log.message }}</span>
          <span
            v-if="isMergeCommit(log)"
            class="shrink-0 inline-flex items-center gap-0.5 px-1.5 py-0.5 rounded text-[9px] font-mono-ui bg-amber-500/15 text-amber-600 dark:text-amber-400"
            title="合并提交"
          >
            <GitMerge :size="9" />
            合并
          </span>
          <span
            v-for="ref in log.refs"
            :key="ref"
            class="shrink-0 px-1.5 py-0.5 rounded text-[9px] font-mono-ui bg-[--accent]/15 text-[--accent] transition-colors pointer-events-auto"
            :class="branchTipNames.has(ref) ? 'hover:bg-[--accent] hover:text-white cursor-pointer' : ''"
            :title="branchTipNames.has(ref) ? '只看这个分支泳道' : ref"
            @click.stop="onRefClick(ref)"
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
          <span class="flex items-center gap-1" :title="log.date">
            <CalendarDays :size="10" />
            {{ formatCommitDate(log.date) }}
          </span>
        </div>
      </div>
    </div>

    <div
      class="absolute top-0 left-0 z-[3] flex h-full cursor-pointer"
      :style="{ width: `${graphWidth}px` }"
      @click="onGraphClick"
      @mousemove="onGraphMouseMove"
    >
      <div
        v-for="column in laneCount"
        :key="`lane-hit-${column - 1}`"
        class="h-full"
        :style="{ width: `${GRAPH_LANE_WIDTH}px` }"
        @mouseenter="onLaneEnter(column - 1)"
        @mouseleave="onLaneLeave"
      />
    </div>

    <div
      v-if="hoveredLaneLabel"
      class="absolute z-[4] pointer-events-none max-w-[220px] px-2 py-1 rounded-[var(--radius)] text-[10px] leading-snug text-[--text-primary] bg-[--bg-tertiary] border border-[--border-color] shadow-md truncate"
      :style="{
        left: `${(hoveredLaneColumn ?? 0) * GRAPH_LANE_WIDTH + GRAPH_LANE_WIDTH / 2}px`,
        top: '6px',
        transform: 'translateX(-50%)',
      }"
    >
      {{ hoveredLaneLabel }}
    </div>

    <Teleport to="body">
      <div
        v-if="contextMenu"
        class="fixed z-[9999] bg-[--bg-tertiary] border border-[--border-color] rounded-[var(--radius)] shadow-lg p-2 text-xs min-w-[160px]"
        :style="{ left: contextMenu.x + 'px', top: contextMenu.y + 'px' }"
        @click.stop
      >
        <button
          class="w-full text-left px-2.5 py-2 rounded-[var(--radius)] text-[--text-primary] hover:bg-[--accent] hover:text-white transition-colors cursor-pointer"
          @click="runCommitAction('cherryPick')"
        >Cherry-pick</button>
        <button
          class="w-full text-left px-2.5 py-2 rounded-[var(--radius)] text-[--text-primary] hover:bg-[--accent] hover:text-white transition-colors cursor-pointer"
          @click="runCommitAction('revert')"
        >Revert commit</button>
        <button
          class="w-full text-left px-2.5 py-2 rounded-[var(--radius)] text-[--text-primary] hover:bg-[--accent] hover:text-white transition-colors cursor-pointer"
          @click="runCommitAction('createBranchHere')"
        >Create branch here</button>
      </div>
    </Teleport>
  </div>
</template>
