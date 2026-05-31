<script setup lang="ts">
import { ref, computed, watch, onMounted, onUnmounted } from 'vue'
import { invoke } from '@tauri-apps/api/core'
import { ChevronDown, ChevronRight, FileCode, FilePlus, Undo2, User, CalendarDays } from 'lucide-vue-next'
import { diffShowsNewFile } from '../utils/gitStatus'
import {
  buildPatchForSegment,
  buildPatchForSelection,
  getHunkBodySegments,
  lineKeysInChangeSegment,
  parseDiffSections,
  type DiffHunk,
  type DiffLine,
  type HunkBodySegment,
} from '../utils/diffPatch'
import {
  applyLineSelectionClick,
  lineRangeSelection,
  type SelectableLineRef,
} from '../utils/diffLineSelection'

const props = defineProps<{
  diffText: string
  fileName: string | null
  canStage: boolean
  canRevert: boolean
  filePath: string | null
  /** 仓库根路径，用于加载二进制图片预览 */
  repoPath: string | null
  /** 当前工作区选中文件是否在已暂存列表（`git diff --cached` vs `git diff`） */
  workspaceIsStaged: boolean
  /** 历史里查看某条提交时传入 commit hash，否则为 null */
  commitHash: string | null
  /** patch 操作中（stage/revert + refresh 未完成），此时禁用所有 Stage/Revert 按钮 */
  patchStaging?: boolean
}>()

const emit = defineEmits<{
  stageFile: [path: string]
  stagePatch: [patch: string]
  revertFile: [path: string, isStaged: boolean]
  /** 第二个参数与当前展示的 diff 一致（Staged / Unstaged） */
  revertPatch: [patch: string, isStaged: boolean]
}>()

// Collapsed state per file section (indexed by section index in sections array)
const collapsedSections = ref<Record<number, boolean>>({})
function isSectionCollapsed(idx: number): boolean {
  return collapsedSections.value[idx] ?? false
}
function toggleSection(idx: number) {
  collapsedSections.value = { ...collapsedSections.value, [idx]: !isSectionCollapsed(idx) }
}

// Collapsed state per hunk (keyed by "sectionIdx:hunkIdx")
const collapsedHunks = ref<Record<string, boolean>>({})
function isHunkCollapsed(sectionIdx: number, hunkIdx: number): boolean {
  return collapsedHunks.value[`${sectionIdx}:${hunkIdx}`] ?? false
}
function toggleHunk(sectionIdx: number, hunkIdx: number) {
  const key = `${sectionIdx}:${hunkIdx}`
  collapsedHunks.value = { ...collapsedHunks.value, [key]: !isHunkCollapsed(sectionIdx, hunkIdx) }
}

// Line selection (gutter): order for shift-range and copy
const selectedLineIds = ref<Set<string>>(new Set())
/** Anchor for Shift+click range (last non-shift gutter click) */
const anchorLineId = ref<string | null>(null)
const isDraggingLineSelection = ref(false)

// ── Commit info ──

interface CommitInfo {
  hash: string
  author: string
  date: string
  message: string
}

/** 新文件 diff 不支持按块 Revert（仅 + 行，语义是删文件） */
const showHunkRevert = computed(
  () => props.canRevert && !diffShowsNewFile(props.diffText),
)

const commitInfo = computed((): CommitInfo | null => {
  const text = props.diffText
  if (!text) return null
  if (!text.startsWith('commit ')) return null

  const lines = text.split('\n')
  const hash = lines[0].slice(7).trim()
  let author = ''
  let date = ''
  let message = ''
  let i = 1

  while (i < lines.length && !lines[i].startsWith('diff ')) {
    const line = lines[i]
    if (line.startsWith('Author: ')) {
      author = line.slice(8).trim()
    } else if (line.startsWith('Date:   ')) {
      date = line.slice(8).trim()
    } else if (line.startsWith('    ')) {
      message += (message ? '\n' : '') + line.trim()
    }
    i++
  }

  return { hash, author, date, message }
})

// ── Parse sections + hunks ──

const sections = computed(() => parseDiffSections(props.diffText, props.fileName))

const IMAGE_FILE_RE = /\.(png|jpe?g|gif|webp|bmp|ico)$/i

function isImageFileName(name: string): boolean {
  return IMAGE_FILE_RE.test(name)
}

/** 无 @@ 块、且为二进制差异的图片文件（git 对二进制不输出文本 hunk） */
const binaryImageEntries = computed((): { fileName: string }[] => {
  const text = props.diffText
  if (!text) return []

  const diffStart = text.indexOf('\ndiff --git ')
  const diffContent = diffStart >= 0 ? text.slice(diffStart + 1) : text
  const rawParts = diffContent.split('\ndiff --git ')
  const result: { fileName: string }[] = []

  for (let idx = 0; idx < rawParts.length; idx++) {
    const part = idx === 0 ? rawParts[0] : rawParts[idx]
    if (!part.trim()) continue

    const fullText = idx === 0 ? part : 'diff --git ' + part
    const allLines = fullText.split('\n')

    const firstLine = allLines[0]
    const match = firstLine.match(/diff --git a\/(.+) b\/(.+)/)
    const fileName = match ? (match[2] || match[1]) : (props.fileName || '')
    if (!fileName || !isImageFileName(fileName)) continue

    const hasHunk = allLines.some((l) => l.startsWith('@@'))
    if (hasHunk) continue

    const hasBinary = allLines.some(
      (l) => l.startsWith('Binary files ') || l.includes('GIT binary patch'),
    )
    if (!hasBinary) continue

    result.push({ fileName })
  }

  return result
})

interface BinaryImagePreviewRow {
  fileName: string
  oldDataUrl: string | null
  newDataUrl: string | null
}

const binaryImagePreviewRows = ref<BinaryImagePreviewRow[]>([])
const binaryImagePreviewLoading = ref(false)
let binaryPreviewRequestId = 0

watch(
  () =>
    [
      props.diffText,
      props.repoPath,
      props.filePath,
      props.workspaceIsStaged,
      props.commitHash,
    ] as const,
  async () => {
    const entries = binaryImageEntries.value
    binaryImagePreviewRows.value = []
    if (!props.repoPath || entries.length === 0) {
      binaryImagePreviewLoading.value = false
      return
    }

    const kind = props.commitHash ? 'commit' : props.workspaceIsStaged ? 'staged' : 'unstaged'
    const paths =
      props.commitHash != null
        ? entries.map((e) => e.fileName)
        : props.filePath
          ? entries.filter((e) => e.fileName === props.filePath).map((e) => e.fileName)
          : entries.map((e) => e.fileName)

    if (paths.length === 0) {
      binaryImagePreviewLoading.value = false
      return
    }

    const req = ++binaryPreviewRequestId
    binaryImagePreviewLoading.value = true
    try {
      const rows: BinaryImagePreviewRow[] = []
      for (const relativePath of paths) {
        const preview = await invoke<{
          oldDataUrl: string | null
          newDataUrl: string | null
        }>('get_binary_image_preview', {
          relativePath,
          kind,
          commitHash: props.commitHash,
        })
        rows.push({
          fileName: relativePath,
          oldDataUrl: preview.oldDataUrl ?? null,
          newDataUrl: preview.newDataUrl ?? null,
        })
      }
      if (req === binaryPreviewRequestId) {
        binaryImagePreviewRows.value = rows
      }
    } catch {
      if (req === binaryPreviewRequestId) {
        binaryImagePreviewRows.value = []
      }
    } finally {
      if (req === binaryPreviewRequestId) {
        binaryImagePreviewLoading.value = false
      }
    }
  },
  { immediate: true },
)

// ── Stage helpers ──

/** Stage the entire file */
function stageEntireFile(path: string) {
  emit('stageFile', path)
}

function revertEntireFile(path: string) {
  emit('revertFile', path, props.workspaceIsStaged)
}

// ── Line selection + change blocks ──

const selectableFlat = computed(() => {
  const out: (SelectableLineRef & { si: number; hi: number; li: number })[] = []
  for (let si = 0; si < sections.value.length; si++) {
    const section = sections.value[si]
    for (let hi = 0; hi < section.hunks.length; hi++) {
      const hunk = section.hunks[hi]
      for (let li = 0; li < hunk.lines.length; li++) {
        const line = hunk.lines[li]
        if (line.type === 'added' || line.type === 'removed') out.push({ id: line.key, si, hi, li })
      }
    }
  }
  return out
})

function flatIndexOf(id: string): number {
  return selectableFlat.value.findIndex((x) => x.id === id)
}

function changeLineIdsInBlock(hunk: DiffHunk, blk: HunkBodySegment): Set<string> {
  return lineKeysInChangeSegment(hunk, blk)
}

function selectedLineIdsInBlock(hunk: DiffHunk, blk: HunkBodySegment): Set<string> {
  const blockIds = changeLineIdsInBlock(hunk, blk)
  const selected = new Set<string>()
  for (const id of selectedLineIds.value) {
    if (blockIds.has(id)) selected.add(id)
  }
  return selected
}

/** 选中行与 +/- hover 背景互斥，避免 hover 时盖住蓝色。 */
function changeLineRowClass(line: DiffLine | undefined, selected: boolean): Record<string, boolean> {
  if (!line || line.type === 'header' || line.type === 'noNewline') return {}
  if (selected) {
    return {
      'relative z-[2] !bg-blue-700/80 hover:!bg-blue-600/90 text-blue-50 ring-1 ring-inset ring-blue-300/90': true,
    }
  }
  return {
    'bg-[--diff-added] hover:bg-green-800/40': line.type === 'added',
    'bg-[--diff-removed] hover:bg-red-800/40': line.type === 'removed',
    'hover:bg-[--bg-tertiary]': line.type === 'context',
    'text-[--text-primary]': line.type === 'context',
    'text-[--diff-added-text]': line.type === 'added',
    'text-[--diff-removed-text]': line.type === 'removed',
  }
}

/** 闭区间下标；可选 maxIdx 防止越界。 */
function liRange(start: number, end: number, maxIdx?: number): number[] {
  if (maxIdx != null && maxIdx < 0) return []
  let lo = Math.min(start, end)
  let hi = Math.max(start, end)
  if (maxIdx != null) {
    lo = Math.max(0, Math.min(lo, maxIdx))
    hi = Math.max(0, Math.min(hi, maxIdx))
  }
  if (lo > hi) return []
  const r: number[] = []
  for (let i = lo; i <= hi; i++) r.push(i)
  return r
}

function stageChangeBlock(sectionIdx: number, hunkIdx: number, blk: HunkBodySegment) {
  const section = sections.value[sectionIdx]
  const hunk = section.hunks[hunkIdx]
  const patch = buildPatchForSegment(section, hunk, blk, 'stage')
  if (patch) emit('stagePatch', patch)
}

function revertChangeBlock(sectionIdx: number, hunkIdx: number, blk: HunkBodySegment) {
  const section = sections.value[sectionIdx]
  const hunk = section.hunks[hunkIdx]
  const patch = buildPatchForSegment(section, hunk, blk, 'revert')
  if (patch) emit('revertPatch', patch, props.workspaceIsStaged)
}

function stageSelectedLinesInBlock(sectionIdx: number, hunkIdx: number, blk: HunkBodySegment) {
  const section = sections.value[sectionIdx]
  const hunk = section.hunks[hunkIdx]
  const ids = selectedLineIdsInBlock(hunk, blk)
  const patch = buildPatchForSelection([section], ids, 'stage')
  if (patch) emit('stagePatch', patch)
  clearLineSelection()
}

function revertSelectedLinesInBlock(sectionIdx: number, hunkIdx: number, blk: HunkBodySegment) {
  const section = sections.value[sectionIdx]
  const hunk = section.hunks[hunkIdx]
  const ids = selectedLineIdsInBlock(hunk, blk)
  const patch = buildPatchForSelection([section], ids, 'revert')
  if (patch) emit('revertPatch', patch, props.workspaceIsStaged)
  clearLineSelection()
}

function selectDiffLine(e: MouseEvent, lineId: string) {
  e.stopPropagation()
  e.preventDefault()
  const result = applyLineSelectionClick({
    lines: selectableFlat.value,
    current: selectedLineIds.value,
    clickedId: lineId,
    anchorId: anchorLineId.value,
    shiftKey: e.shiftKey,
    toggleKey: e.ctrlKey || e.metaKey,
  })
  selectedLineIds.value = result.selected
  anchorLineId.value = result.anchorId
  isDraggingLineSelection.value = !e.shiftKey && !(e.ctrlKey || e.metaKey)
}

function onDiffLineMouseDown(e: MouseEvent, lineId: string | undefined) {
  if (!lineId || e.button !== 0) return
  selectDiffLine(e, lineId)
}

function onDiffLineMouseEnter(lineId: string | undefined) {
  if (!lineId || !isDraggingLineSelection.value || anchorLineId.value == null) return
  selectedLineIds.value = lineRangeSelection(selectableFlat.value, anchorLineId.value, lineId)
}

function clearLineSelection() {
  selectedLineIds.value = new Set()
  anchorLineId.value = null
}

watch(() => props.diffText, () => clearLineSelection())

function stopLineSelectionDrag() {
  isDraggingLineSelection.value = false
}

function onDiffSurfacePointerDown(e: MouseEvent) {
  const t = e.target as HTMLElement | null
  if (!t) return
  if (t.closest('.diff-select-zone') || t.closest('.diff-stage-float') || t.closest('.diff-revert-float')) return
  clearLineSelection()
}

function orderedSelectedLinesText(): string {
  const ids = selectedLineIds.value
  if (ids.size === 0) return ''
  const lines: string[] = []
  for (const row of selectableFlat.value) {
    if (ids.has(row.id)) {
      const line = sections.value[row.si].hunks[row.hi].lines[row.li]
      lines.push(line.content)
    }
  }
  return lines.join('\n')
}

function onGlobalKeyDown(e: KeyboardEvent) {
  if (!(e.ctrlKey || e.metaKey) || e.key !== 'c') return
  const sel = window.getSelection()?.toString() ?? ''
  if (sel.length > 0) return
  if (selectedLineIds.value.size === 0) return
  e.preventDefault()
  void navigator.clipboard.writeText(orderedSelectedLinesText())
}

onMounted(() => {
  window.addEventListener('keydown', onGlobalKeyDown)
  window.addEventListener('mouseup', stopLineSelectionDrag)
})
onUnmounted(() => {
  window.removeEventListener('keydown', onGlobalKeyDown)
  window.removeEventListener('mouseup', stopLineSelectionDrag)
})

</script>

<template>
  <div class="h-full flex flex-col bg-[--bg-primary]">
    <!-- Diff header bar -->
    <div class="px-2.5 py-2.5 text-xs text-[--text-secondary] border-b border-[--border-color] flex items-center gap-2 flex-shrink-0 min-w-0">
      <span v-if="fileName" class="text-[--text-primary] font-medium truncate font-mono-ui min-w-0 flex-1">{{ fileName }}</span>
      <span v-else class="text-[--text-secondary] shrink-0">选择文件查看差异</span>
      <span v-if="sections.length > 1" class="text-[10px] text-[--text-secondary] font-mono-ui shrink-0">{{ sections.length }} 个文件</span>
    </div>

    <!-- Diff content -->
    <div class="flex-1 overflow-y-auto" @mousedown="onDiffSurfacePointerDown">
      <div v-if="!diffText" class="p-2.5 text-[--text-secondary] text-xs">
        <template v-if="fileName">没有差异内容</template>
        <template v-else>点击左侧文件查看变更</template>
      </div>

      <template v-else>
        <!-- Commit info (from git show) -->
        <div v-if="commitInfo" class="px-2.5 py-2.5 border-b border-[--border-color] bg-[--bg-secondary]">
          <div class="flex items-center gap-2 mb-2">
            <span class="text-xs font-mono-ui text-[--accent] font-medium">{{ commitInfo.hash }}</span>
          </div>
          <div class="flex items-center flex-wrap gap-x-3 gap-y-1 text-[10px] text-[--text-secondary] mb-2">
            <span class="flex items-center gap-1">
              <User :size="12" />
              {{ commitInfo.author }}
            </span>
            <span class="flex items-center gap-1">
              <CalendarDays :size="12" />
              {{ commitInfo.date }}
            </span>
          </div>
          <div class="text-xs text-[--text-primary] font-medium leading-relaxed whitespace-pre-wrap">
            {{ commitInfo.message }}
          </div>
        </div>

        <!-- 二进制图片：在 diff 区域并排预览（git 不输出文本 hunk） -->
        <div
          v-if="binaryImageEntries.length > 0"
          class="border-b border-[--border-color] bg-[--bg-secondary]"
        >
          <div class="px-2.5 py-2.5 text-[10px] text-[--text-secondary] border-b border-[--border-color] uppercase tracking-wide">
            图片二进制差异
            <span v-if="binaryImagePreviewLoading" class="ml-2 text-[--accent]">加载预览中…</span>
          </div>
          <div v-if="!repoPath" class="px-2.5 py-2.5 text-xs text-[--text-secondary]">
            未打开仓库路径，无法预览图片。
          </div>
          <template v-else>
            <div
              v-for="row in binaryImagePreviewRows"
              :key="row.fileName"
              class="px-2.5 py-2.5 border-b border-[--border-color] last:border-b-0"
            >
              <div class="text-xs font-medium text-[--text-primary] mb-2 truncate font-mono-ui">{{ row.fileName }}</div>
              <div class="flex flex-wrap gap-2.5">
                <div class="flex-1 min-w-[120px] max-w-full sm:max-w-[calc(50%-0.3125rem)]">
                  <div class="text-[10px] uppercase tracking-wide text-[--text-secondary] mb-2">变更前</div>
                  <img
                    v-if="row.oldDataUrl"
                    :src="row.oldDataUrl"
                    alt="变更前"
                    class="max-h-56 w-auto max-w-full rounded-[var(--radius)] border border-[--border-color] object-contain bg-[--bg-tertiary]"
                  />
                  <div v-else class="text-xs text-[--text-secondary] p-2.5 text-center rounded-[var(--radius)] border border-dashed border-[--border-color] bg-[--bg-tertiary]/50">
                    无旧版（如新增文件）
                  </div>
                </div>
                <div class="flex-1 min-w-[120px] max-w-full sm:max-w-[calc(50%-0.3125rem)]">
                  <div class="text-[10px] uppercase tracking-wide text-[--text-secondary] mb-2">变更后</div>
                  <img
                    v-if="row.newDataUrl"
                    :src="row.newDataUrl"
                    alt="变更后"
                    class="max-h-56 w-auto max-w-full rounded-[var(--radius)] border border-[--border-color] object-contain bg-[--bg-tertiary]"
                  />
                  <div v-else class="text-xs text-[--text-secondary] p-2.5 text-center rounded-[var(--radius)] border border-dashed border-[--border-color] bg-[--bg-tertiary]/50">
                    无新版（如已删除）
                  </div>
                </div>
              </div>
            </div>
          </template>
        </div>

        <!-- File diff sections -->
        <div
          v-for="(section, si) in sections"
          :key="si"
          class="border-b border-[--border-color]"
        >
          <!-- File header bar -->
          <div
            class="sticky top-0 z-10 flex min-w-0 items-center gap-2 px-2.5 py-2.5 bg-[--bg-tertiary] border-b border-[--border-color] cursor-pointer select-none"
            @click="toggleSection(si)"
          >
            <ChevronDown v-if="!isSectionCollapsed(si)" :size="14" class="flex-shrink-0 text-[--text-secondary]" />
            <ChevronRight v-else :size="14" class="flex-shrink-0 text-[--text-secondary]" />
            <FileCode :size="14" class="flex-shrink-0 text-[--text-secondary]" />
            <span class="min-w-0 flex-1 truncate text-xs text-[--text-primary] font-medium font-mono-ui">{{ section.fileName || '差异' }}</span>
            <div class="ml-2 flex shrink-0 items-center gap-2">
              <button
                v-if="showHunkRevert"
                :disabled="patchStaging"
                class="flex items-center gap-1 px-2.5 py-2.5 rounded-[var(--radius)] text-[10px] bg-orange-700/70 hover:bg-orange-600 text-white transition-colors cursor-pointer disabled:opacity-40 disabled:cursor-not-allowed"
                title="丢弃整个文件变更"
                @click.stop="revertEntireFile(filePath || section.fileName)"
              >
                <Undo2 :size="12" />
                Revert 文件
              </button>
              <button
                v-if="canStage"
                :disabled="patchStaging"
                class="flex items-center gap-1 px-2.5 py-2.5 rounded-[var(--radius)] text-[10px] bg-green-700/70 hover:bg-green-600 text-white transition-colors cursor-pointer disabled:opacity-40 disabled:cursor-not-allowed"
                title="Stage 整个文件"
                @click.stop="stageEntireFile(filePath || section.fileName)"
              >
                <FilePlus :size="12" />
                Stage 文件
              </button>
              <span class="text-[10px] text-[--diff-added-text] font-mono-ui">{{ section.hunks.reduce((s, h) => s + h.lines.filter(l => l.type === 'added').length, 0) }} +</span>
              <span class="text-[10px] text-[--diff-removed-text] font-mono-ui">{{ section.hunks.reduce((s, h) => s + h.lines.filter(l => l.type === 'removed').length, 0) }} -</span>
            </div>
          </div>

          <!-- Hunks -->
          <div v-if="!isSectionCollapsed(si)">
            <div
              v-for="(hunk, hi) in section.hunks"
              :key="`${si}-${hi}`"
            >
              <!-- Hunk header -->
              <div
                class="sticky top-12 z-[5] flex min-w-0 items-center gap-2 px-2.5 py-2.5 bg-[--bg-secondary] border-b border-[--border-color] cursor-pointer select-none group"
                :class="{ 'opacity-60': isHunkCollapsed(si, hi) }"
                @click="toggleHunk(si, hi)"
              >
                <ChevronDown v-if="!isHunkCollapsed(si, hi)" :size="12" class="flex-shrink-0 text-[--text-secondary]" />
                <ChevronRight v-else :size="12" class="flex-shrink-0 text-[--text-secondary]" />
                <span class="min-w-0 flex-1 truncate text-[10px] font-mono-ui text-[--text-secondary]">{{ hunk.header }}</span>
              </div>

              <!-- Hunk lines: @@ header + sub-blocks + gutter selection -->
              <div v-if="!isHunkCollapsed(si, hi)" class="font-mono-ui text-xs leading-relaxed border-b border-[--border-color]">
                <div
                  v-if="hunk.lines[0]?.type === 'header'"
                  class="flex text-[--text-secondary] transition-colors hover:bg-[--bg-tertiary]"
                >
                  <div class="flex-shrink-0 w-3" />
                  <div class="flex-shrink-0 w-7 pr-1 select-none" />
                  <div class="flex-shrink-0 w-7 pr-2.5 select-none" />
                  <span class="px-2.5 whitespace-pre-wrap flex-1 min-w-0 select-text">{{ hunk.lines[0].content }}</span>
                </div>
                <template v-for="(seg, bidx) in getHunkBodySegments(hunk)" :key="`${si}-${hi}-${bidx}-${seg.kind}`">
                  <div
                    v-if="seg.kind === 'changes'"
                    class="relative group/diffblk rounded-sm"
                  >
                    <div
                      v-if="showHunkRevert || canStage"
                      class="flex min-w-0 items-center gap-2 border-y border-[--border-color] bg-[--bg-secondary] px-2.5 py-1.5 text-[10px] text-[--text-secondary] select-none"
                    >
                      <span class="min-w-0 flex-1 truncate font-mono-ui">
                        {{ selectedLineIdsInBlock(hunk, seg).size > 0 ? `已选 ${selectedLineIdsInBlock(hunk, seg).size} 行` : '变更区块' }}
                      </span>
                      <button
                        v-if="showHunkRevert"
                        :disabled="patchStaging"
                        type="button"
                        class="diff-revert-float flex items-center gap-1 px-2 py-1 rounded-[var(--radius)] text-[10px] bg-orange-700 text-white shadow-md cursor-pointer disabled:opacity-40 disabled:cursor-not-allowed"
                        :title="selectedLineIdsInBlock(hunk, seg).size > 0 ? '放弃当前区块内选中的行' : '放弃本段连续 +/- 行'"
                        @mousedown.stop
                        @click.stop="selectedLineIdsInBlock(hunk, seg).size > 0 ? revertSelectedLinesInBlock(si, hi, seg) : revertChangeBlock(si, hi, seg)"
                      >
                        <Undo2 :size="12" />
                        {{ selectedLineIdsInBlock(hunk, seg).size > 0 ? '放弃行' : '放弃区块' }}
                      </button>
                      <button
                        v-if="canStage"
                        :disabled="patchStaging"
                        type="button"
                        class="diff-stage-float flex items-center gap-1 px-2 py-1 rounded-[var(--radius)] text-[10px] bg-green-700 text-white shadow-md cursor-pointer disabled:opacity-40 disabled:cursor-not-allowed"
                        :title="selectedLineIdsInBlock(hunk, seg).size > 0 ? '暂存当前区块内选中的行' : '暂存本段连续 +/- 行'"
                        @mousedown.stop
                        @click.stop="selectedLineIdsInBlock(hunk, seg).size > 0 ? stageSelectedLinesInBlock(si, hi, seg) : stageChangeBlock(si, hi, seg)"
                      >
                        <FilePlus :size="12" />
                        {{ selectedLineIdsInBlock(hunk, seg).size > 0 ? '暂存行' : '暂存区块' }}
                      </button>
                    </div>
                    <div
                      v-for="li in liRange(seg.startLineIndex, seg.endLineIndex, hunk.lines.length - 1)"
                      :key="`${si}-${hi}-${li}-${hunk.lines[li]?.key ?? 'x'}`"
                      class="relative flex transition-colors select-none"
                      :class="[
                        { 'text-[--text-secondary]': hunk.lines[li]?.type === 'header' },
                        changeLineRowClass(
                          hunk.lines[li],
                          !!(hunk.lines[li] && selectedLineIds.has(hunk.lines[li].key)),
                        ),
                      ]"
                      :data-selected="hunk.lines[li] && selectedLineIds.has(hunk.lines[li].key) ? '' : undefined"
                      @mousedown="onDiffLineMouseDown($event, hunk.lines[li]?.key)"
                      @mouseenter="onDiffLineMouseEnter(hunk.lines[li]?.key)"
                    >
                      <div
                        v-if="hunk.lines[li]?.type !== 'header'"
                        class="diff-select-zone flex flex-shrink-0 cursor-pointer select-none"
                      >
                        <div
                          class="flex-shrink-0 w-3 flex items-center justify-center text-[10px]"
                          :class="hunk.lines[li] && selectedLineIds.has(hunk.lines[li].key) ? 'text-blue-300' : 'text-transparent'"
                        >{{ hunk.lines[li] && selectedLineIds.has(hunk.lines[li].key) ? '✓' : '○' }}</div>
                        <div class="flex-shrink-0 w-7 text-right pr-1 text-[--text-secondary]/35">{{ hunk.lines[li]?.oldLineNumber ?? '' }}</div>
                        <div class="flex-shrink-0 w-7 text-right pr-2.5 text-[--text-secondary]/50">{{ hunk.lines[li]?.newLineNumber ?? '' }}</div>
                      </div>
                      <template v-else>
                        <div class="flex-shrink-0 w-3" />
                        <div class="flex-shrink-0 w-7 pr-1 select-none" />
                        <div class="flex-shrink-0 w-7 pr-2.5 select-none" />
                      </template>
                      <span class="px-2.5 whitespace-pre-wrap flex-1 min-w-0">{{ hunk.lines[li]?.content }}</span>
                    </div>
                    <!-- 叠在行上方：不挡点击/划选；hover 整块时可见描边 + 淡绿罩 -->
                    <div
                      aria-hidden="true"
                      class="pointer-events-none absolute inset-0 z-[1] rounded-sm border-2 border-transparent opacity-0 transition-[opacity,border-color,background-color] duration-150 group-hover/diffblk:border-green-500/70 group-hover/diffblk:bg-green-500/15 group-hover/diffblk:opacity-100 group-has-[[data-selected]:hover]/diffblk:opacity-0 group-has-[[data-selected]:hover]/diffblk:border-transparent"
                    />
                  </div>
                  <template v-else>
                    <div
                      v-for="li in liRange(seg.startLineIndex, seg.endLineIndex, hunk.lines.length - 1)"
                      :key="`${si}-${hi}-${li}-${hunk.lines[li]?.key ?? 'x'}`"
                      class="flex transition-colors"
                      :class="{
                        'bg-[--diff-added] hover:bg-green-800/40': hunk.lines[li]?.type === 'added',
                        'bg-[--diff-removed] hover:bg-red-800/40': hunk.lines[li]?.type === 'removed',
                        'hover:bg-[--bg-tertiary]': hunk.lines[li]?.type === 'context',
                        'text-[--text-secondary]': hunk.lines[li]?.type === 'header',
                        'text-[--text-primary]': hunk.lines[li]?.type === 'context',
                        'text-[--diff-added-text]': hunk.lines[li]?.type === 'added',
                        'text-[--diff-removed-text]': hunk.lines[li]?.type === 'removed',
                        'outline outline-1 outline-green-500/60 -outline-offset-1': hunk.lines[li] && selectedLineIds.has(hunk.lines[li].key),
                      }"
                    >
                      <div
                        v-if="hunk.lines[li]?.type !== 'header'"
                        class="flex flex-shrink-0 select-none"
                      >
                        <div
                          class="flex-shrink-0 w-3 flex items-center justify-center text-[10px] text-transparent"
                        >○</div>
                        <div class="flex-shrink-0 w-7 text-right pr-1 text-[--text-secondary]/35">{{ hunk.lines[li]?.oldLineNumber ?? '' }}</div>
                        <div class="flex-shrink-0 w-7 text-right pr-2.5 text-[--text-secondary]/50">{{ hunk.lines[li]?.newLineNumber ?? '' }}</div>
                      </div>
                      <template v-else>
                        <div class="flex-shrink-0 w-3" />
                        <div class="flex-shrink-0 w-7 pr-1 select-none" />
                        <div class="flex-shrink-0 w-7 pr-2.5 select-none" />
                      </template>
                      <span class="px-2.5 whitespace-pre-wrap flex-1 min-w-0 select-text">{{ hunk.lines[li]?.content }}</span>
                    </div>
                  </template>
                </template>
              </div>
            </div>
          </div>
        </div>
      </template>
    </div>
  </div>
</template>
