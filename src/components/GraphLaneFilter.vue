<script setup lang="ts">
import { onMounted, onUnmounted, ref } from 'vue'
import { ChevronDown, Check } from 'lucide-vue-next'
import type { GraphLane } from '../utils/commitGraph'

const props = defineProps<{
  lanes: GraphLane[]
  visibleLaneIds: Set<string>
}>()

const emit = defineEmits<{
  toggleLane: [laneId: string]
  showAll: []
  showMainOnly: []
}>()

const open = ref(false)

function isVisible(laneId: string): boolean {
  return props.visibleLaneIds.has(laneId)
}

function visibleCount(): number {
  return props.lanes.filter((lane) => isVisible(lane.id)).length
}

function canToggle(laneId: string): boolean {
  if (!isVisible(laneId)) return true
  return visibleCount() > 1
}

function onToggle(laneId: string) {
  if (!canToggle(laneId)) return
  emit('toggleLane', laneId)
}

function close() {
  open.value = false
}

function onDocumentClick(event: MouseEvent) {
  const target = event.target as HTMLElement
  if (!target.closest('.lane-filter')) {
    close()
  }
}

onMounted(() => {
  document.addEventListener('click', onDocumentClick)
})

onUnmounted(() => {
  document.removeEventListener('click', onDocumentClick)
})
</script>

<template>
  <div class="lane-filter relative shrink-0" @click.stop>
    <button
      type="button"
      class="inline-flex items-center gap-1 px-2 py-1 rounded-[var(--radius)] border border-[--border-color] text-[10px] text-[--text-primary] hover:bg-[--bg-tertiary] transition-colors cursor-pointer"
      @click="open = !open"
    >
      泳道 {{ visibleCount() }}/{{ lanes.length }}
      <ChevronDown :size="12" class="transition-transform" :class="open ? 'rotate-180' : ''" />
    </button>

    <div
      v-if="open"
      class="absolute right-0 top-full mt-1 min-w-[200px] max-w-[260px] bg-[--bg-tertiary] border border-[--border-color] rounded-[var(--radius)] shadow-md z-50 py-1"
    >
      <div class="flex items-center justify-between gap-2 px-2.5 py-1.5 border-b border-[--border-color]">
        <span class="text-[10px] text-[--text-secondary]">显示泳道</span>
        <div class="flex items-center gap-2">
          <button
            type="button"
            class="text-[10px] text-[--accent] hover:underline cursor-pointer"
            @click="emit('showAll'); close()"
          >全选</button>
          <button
            type="button"
            class="text-[10px] text-[--accent] hover:underline cursor-pointer"
            @click="emit('showMainOnly'); close()"
          >仅主线</button>
        </div>
      </div>

      <div class="max-h-[220px] overflow-y-auto py-1">
        <button
          v-for="lane in lanes"
          :key="lane.id"
          type="button"
          class="w-full flex items-center gap-2 px-2.5 py-1.5 text-left transition-colors cursor-pointer"
          :class="canToggle(lane.id) || !isVisible(lane.id)
            ? 'hover:bg-[--bg-secondary]'
            : 'opacity-60 cursor-not-allowed'"
          @click="onToggle(lane.id)"
        >
          <span
            class="inline-flex items-center justify-center w-3.5 h-3.5 rounded border shrink-0"
            :class="isVisible(lane.id)
              ? 'bg-[--accent] border-[--accent] text-white'
              : 'border-[--border-color] bg-[--bg-secondary]'"
          >
            <Check v-if="isVisible(lane.id)" :size="10" />
          </span>
          <span
            class="w-2 h-2 rounded-full shrink-0"
            :style="{ backgroundColor: lane.color }"
          />
          <span class="text-[11px] text-[--text-primary] truncate">{{ lane.label }}</span>
        </button>
      </div>
    </div>
  </div>
</template>
