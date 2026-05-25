import { ref, toValue, type MaybeRefOrGetter } from 'vue'
import type { CommitLog } from '../types'
import { layoutCommitGraph, distinctColumnCount } from './commitGraph'
import {
  isGraphHoverSoundEnabled,
  setGraphHoverSoundEnabled,
  unlockGraphHoverSound,
  resetLaneHoverSound,
  playCommitHoverTone,
} from './graphHoverSound'

export function useLaneHoverSound(defaultCommits: MaybeRefOrGetter<CommitLog[]>) {
  const hoverSoundEnabled = ref(isGraphHoverSoundEnabled())

  function toggleHoverSound() {
    hoverSoundEnabled.value = !hoverSoundEnabled.value
    setGraphHoverSoundEnabled(hoverSoundEnabled.value)
  }

  function onCommitHover(hash: string, commits?: CommitLog[]) {
    unlockGraphHoverSound()
    if (!hoverSoundEnabled.value) return

    const list = commits ?? toValue(defaultCommits)
    const layout = layoutCommitGraph(list)
    const column = layout.columnByHash[hash]
    if (column === undefined) return

    const multiLane = distinctColumnCount(layout.columnByHash) > 1
    void playCommitHoverTone(column, hash, { multiLane }).catch(() => {
      // ignore audio errors (autoplay restrictions, etc.)
    })
  }

  function onHoverAreaLeave() {
    resetLaneHoverSound()
  }

  return {
    hoverSoundEnabled,
    toggleHoverSound,
    onCommitHover,
    onHoverAreaLeave,
  }
}
