<script setup lang="ts">
import { computed, ref, toRef, watch } from 'vue'
import { User, CalendarDays, Hash, Loader2, List, GitBranch, GitMerge, Volume2, VolumeX } from 'lucide-vue-next'
import type { BranchTip, CommitAction, CommitLog } from '../types'
import CommitGraphView from './CommitGraphView.vue'
import GraphLaneFilter from './GraphLaneFilter.vue'
import {
  shortHash,
  formatCommitDate,
  isMergeCommit,
  HISTORY_VIEW_STORAGE_KEY,
  laneIdForBranchTip,
} from '../utils/commitGraph'
import { useLaneHoverSound } from '../utils/useLaneHoverSound'
import { useGraphLaneFilter } from '../utils/useGraphLaneFilter'

const props = defineProps<{
  logs: CommitLog[]
  loading: boolean
  loadingMore: boolean
  hasMore: boolean
  selectedHash: string | null
  filter: 'current' | 'all'
  currentBranch: string
  branchTips: BranchTip[]
}>()

const emit = defineEmits<{
  selectCommit: [hash: string]
  commitAction: [payload: { action: CommitAction; hash: string }]
  updateFilter: [filter: 'current' | 'all']
  loadMore: []
}>()

const { hoverSoundEnabled, toggleHoverSound, onCommitHover, onHoverAreaLeave } = useLaneHoverSound(
  toRef(props, 'logs'),
)

const {
  fullLayout,
  lanes,
  visibleLaneIds,
  showLaneFilter,
  displayCommits,
  displayLayout,
  displayLanes,
  toggleLane,
  showAllLanes,
  showMainLaneOnly,
  showLaneOnly,
} = useGraphLaneFilter(toRef(props, 'logs'), toRef(props, 'branchTips'))

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
const graphViewRef = ref<{ clearHover: () => void } | null>(null)
const contextMenu = ref<{ x: number; y: number; hash: string } | null>(null)
const branchTipNames = computed(() => new Set(props.branchTips.map((tip) => tip.name)))

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

function handleHoverAreaLeave() {
  onHoverAreaLeave()
  graphViewRef.value?.clearHover()
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

function focusBranchTip(ref: string) {
  if (!branchTipNames.value.has(ref)) return
  const laneId = laneIdForBranchTip(ref, props.branchTips, fullLayout.value)
  if (!laneId) return
  viewMode.value = 'graph'
  showLaneOnly(laneId)
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
      v-if="logs.length > 0"
      class="shrink-0 flex items-center justify-between gap-2 px-2.5 py-1.5 text-[10px] leading-relaxed text-[--text-secondary] border-b border-[--border-color] bg-[--bg-secondary]"
    >
      <span v-if="viewMode === 'graph'" class="min-w-0">竖线表示提交链，线条汇合为合并。悬停可高亮路径。</span>
      <div
        class="flex items-center gap-2 shrink-0"
        :class="viewMode === 'list' ? 'ml-auto' : ''"
      >
        <button
          type="button"
          class="inline-flex items-center justify-center p-1 rounded-[var(--radius)] text-[--text-secondary] hover:text-[--text-primary] hover:bg-[--bg-tertiary] transition-colors cursor-pointer"
          :title="hoverSoundEnabled ? '关闭泳道音效' : '开启泳道音效'"
          @click="toggleHoverSound"
        >
          <Volume2 v-if="hoverSoundEnabled" :size="12" />
          <VolumeX v-else :size="12" />
        </button>
        <GraphLaneFilter
          v-if="viewMode === 'graph' && showLaneFilter"
          :lanes="lanes"
          :visible-lane-ids="visibleLaneIds"
          @toggle-lane="toggleLane"
          @show-all="showAllLanes"
          @show-main-only="showMainLaneOnly"
        />
      </div>
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
      class="flex-1 min-h-0 overflow-y-auto overflow-x-auto"
      @scroll="onScroll"
      @mouseleave="handleHoverAreaLeave"
    >
      <div v-if="viewMode === 'list'" key="history-list">
        <div
          v-for="log in logs"
          :key="log.hash"
          class="px-2.5 py-2.5 border-b border-[--border-color] cursor-pointer transition-colors"
          :class="log.hash === selectedHash ? 'bg-[--bg-tertiary] border-l-2 border-l-[--accent]' : 'hover:bg-[--bg-tertiary]'"
          @mouseenter="onCommitHover(log.hash)"
          @click="emit('selectCommit', log.hash)"
          @contextmenu="openCommitMenu($event, log.hash)"
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
              class="shrink-0 px-1.5 py-0.5 rounded text-[9px] font-mono-ui bg-[--accent]/15 text-[--accent] transition-colors"
              :class="branchTipNames.has(ref) ? 'hover:bg-[--accent] hover:text-white cursor-pointer' : ''"
              :title="branchTipNames.has(ref) ? '只看这个分支泳道' : ref"
              @click.stop="focusBranchTip(ref)"
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
      </div>

      <CommitGraphView
        v-else
        ref="graphViewRef"
        key="history-graph"
        :logs="displayCommits"
        :graph-layout="displayLayout"
        :display-lanes="displayLanes"
        :branch-tips="branchTips"
        :selected-hash="selectedHash"
        @select-commit="emit('selectCommit', $event)"
        @commit-action="emit('commitAction', $event)"
        @branch-tip-click="focusBranchTip"
        @commit-hover="onCommitHover($event.hash, $event.commits)"
      />

      <div v-if="loadingMore" class="flex items-center justify-center py-3 text-[--text-secondary]">
        <Loader2 :size="14" class="animate-spin mr-2" />
        <span class="text-xs">加载更多...</span>
      </div>
      <div v-else-if="!hasMore && logs.length > 0" class="py-3 text-center text-[10px] text-[--text-secondary]">
        已加载全部
      </div>
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
