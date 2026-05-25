import { computed, ref, toValue, watch, type MaybeRefOrGetter } from 'vue'
import type { BranchTip, CommitLog } from '../types'
import {
  layoutCommitGraph,
  buildGraphLanes,
  buildFilteredGraphLayout,
  syncVisibleLaneIds,
  visibleColumnsFromLanes,
  filterCommitsByColumns,
} from './commitGraph'

function branchTipsByHash(tips: BranchTip[]): Map<string, string[]> {
  const map = new Map<string, string[]>()
  for (const tip of tips) {
    const names = map.get(tip.hash) ?? []
    names.push(tip.name)
    map.set(tip.hash, names)
  }
  return map
}

export function useGraphLaneFilter(
  logs: MaybeRefOrGetter<CommitLog[]>,
  branchTips?: MaybeRefOrGetter<BranchTip[]>,
) {
  const visibleLaneIds = ref<string[]>([])
  const knownLaneIds = ref<string[]>([])

  const fullLayout = computed(() => layoutCommitGraph(toValue(logs)))
  const lanes = computed(() =>
    buildGraphLanes(
      toValue(logs),
      fullLayout.value,
      branchTipsByHash(toValue(branchTips) ?? []),
    ),
  )

  watch(lanes, (nextLanes) => {
    const synced = syncVisibleLaneIds(nextLanes, visibleLaneIds.value, knownLaneIds.value)
    visibleLaneIds.value = synced.visibleLaneIds
    knownLaneIds.value = synced.knownLaneIds
  }, { immediate: true })

  const showLaneFilter = computed(() => lanes.value.length > 1)

  const visibleColumns = computed(() => visibleColumnsFromLanes(lanes.value, visibleLaneIds.value))

  const displayCommits = computed(() =>
    filterCommitsByColumns(toValue(logs), fullLayout.value, visibleColumns.value),
  )

  const displayLayout = computed(() =>
    buildFilteredGraphLayout(toValue(logs), fullLayout.value, visibleColumns.value),
  )

  const displayLanes = computed(() =>
    buildGraphLanes(
      displayCommits.value,
      displayLayout.value,
      branchTipsByHash(toValue(branchTips) ?? []),
    ),
  )

  function toggleLane(laneId: string) {
    const next = new Set(visibleLaneIds.value)
    if (next.has(laneId)) {
      if (next.size <= 1) return
      next.delete(laneId)
    } else {
      next.add(laneId)
    }
    visibleLaneIds.value = [...next]
  }

  function showAllLanes() {
    visibleLaneIds.value = lanes.value.map((lane) => lane.id)
  }

  function showMainLaneOnly() {
    const main = lanes.value.find((lane) => lane.column === 0)
    if (!main) return
    visibleLaneIds.value = [main.id]
  }

  return {
    lanes,
    visibleLaneIds,
    showLaneFilter,
    displayCommits,
    displayLayout,
    displayLanes,
    toggleLane,
    showAllLanes,
    showMainLaneOnly,
  }
}
