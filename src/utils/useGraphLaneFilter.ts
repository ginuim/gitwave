import { computed, ref, toValue, watch, type MaybeRefOrGetter } from 'vue'
import type { CommitLog } from '../types'
import {
  layoutCommitGraph,
  buildGraphLanes,
  syncVisibleLaneIds,
  visibleColumnsFromLanes,
  filterCommitsByColumns,
} from './commitGraph'

export function useGraphLaneFilter(logs: MaybeRefOrGetter<CommitLog[]>) {
  const visibleLaneIds = ref<string[]>([])
  const knownLaneIds = ref<string[]>([])

  const fullLayout = computed(() => layoutCommitGraph(toValue(logs)))
  const lanes = computed(() => buildGraphLanes(toValue(logs), fullLayout.value))

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
    toggleLane,
    showAllLanes,
    showMainLaneOnly,
  }
}
