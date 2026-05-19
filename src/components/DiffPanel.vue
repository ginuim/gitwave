<script setup lang="ts">
import { ref, computed, watch, onMounted, onUnmounted } from 'vue'
import { invoke } from '@tauri-apps/api/core'
import { ChevronDown, ChevronRight, FileCode, FilePlus, Undo2, User, CalendarDays } from 'lucide-vue-next'
import { diffShowsNewFile } from '../utils/gitStatus'

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

interface DiffLine {
  /** `\ No newline at end of file`：不计入 @@ 行数，否则 `git apply` 会报 corrupt patch */
  type: 'header' | 'added' | 'removed' | 'context' | 'noNewline'
  content: string
  /** Unique id for line-selection tracking */
  id: string
}

interface HunkInfo {
  header: string   // e.g. "@@ -1,5 +1,7 @@"
  lines: DiffLine[]
  /** Per-line file line numbers parsed from @@ header; null means "no number" (e.g. header row) */
  lineNums: { old: number | null; new: number | null }[]
  /** 原始 hunk 文本（与 git diff 输出一致，供 apply/revert 使用） */
  rawText: string
}

interface FileDiffSection {
  fileName: string
  diffPrefix: string   // "diff --git a/path b/path\n--- a/path\n+++ b/path"
  hunks: HunkInfo[]
}

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

let lineIdCounter = 0
function nextLineId(): string {
  return `l${lineIdCounter++}`
}

const sections = computed((): FileDiffSection[] => {
  const text = props.diffText
  if (!text) return []

  // Find start of actual diffs
  const diffStart = text.indexOf('\ndiff --git ')
  const diffContent = diffStart >= 0 ? text.slice(diffStart + 1) : text

  const rawParts = diffContent.split('\ndiff --git ')
  const result: FileDiffSection[] = []

  lineIdCounter = 0

  for (let idx = 0; idx < rawParts.length; idx++) {
    const part = idx === 0 ? rawParts[0] : rawParts[idx]
    if (!part.trim()) continue

    const fullText = idx === 0 ? part : 'diff --git ' + part
    const allLines = fullText.split('\n')

    // Extract file name from diff --git line
    let fileName = ''
    const firstLine = allLines[0]
    const match = firstLine.match(/diff --git a\/(.+) b\/(.+)/)
    fileName = match ? (match[2] || match[1]) : (props.fileName || '')

    const diffGitLine = allLines[0]
    const minusIdx = allLines.findIndex(l => l.startsWith('--- '))
    const plusIdx = allLines.findIndex(l => l.startsWith('+++ '))
    const metaLines = minusIdx > 1 ? allLines.slice(1, minusIdx) : []
    const minusLine = minusIdx >= 0 ? allLines[minusIdx] : `--- a/${fileName}`
    const plusLine = plusIdx >= 0 ? allLines[plusIdx] : `+++ b/${fileName}`
    const diffPrefix = [diffGitLine, ...metaLines, minusLine, plusLine].join('\n')

    // Find all @@ lines
    const hunkStartIndices: number[] = []
    for (let i = 0; i < allLines.length; i++) {
      if (allLines[i].startsWith('@@')) {
        hunkStartIndices.push(i)
      }
    }

    if (hunkStartIndices.length === 0) continue

    const hunks: HunkInfo[] = []
    for (let h = 0; h < hunkStartIndices.length; h++) {
      const start = hunkStartIndices[h]
      const end = h + 1 < hunkStartIndices.length ? hunkStartIndices[h + 1] : allLines.length

      const hunkLines: DiffLine[] = []
      for (let li = start; li < end; li++) {
        const line = allLines[li]
        let type: DiffLine['type']
        if (line.startsWith('@@')) type = 'header'
        else if (line.startsWith('+')) type = 'added'
        else if (line.startsWith('-')) type = 'removed'
        else if (line.startsWith('\\')) type = 'noNewline'
        else type = 'context'

        hunkLines.push({ type, content: line, id: nextLineId() })
      }

      // Compute per-line file line numbers from @@ header
      const headerLine = allLines[start]
      const headerMatch = headerLine.match(/^@@ -(\d+)(?:,\d+)? \+(\d+)(?:,\d+)? @@/)
      const lineNums: HunkInfo['lineNums'] = []
      let oldLine = headerMatch ? parseInt(headerMatch[1]) : 1
      let newLine = headerMatch ? parseInt(headerMatch[2]) : 1
      for (const dl of hunkLines) {
        if (dl.type === 'header' || dl.type === 'noNewline') {
          lineNums.push({ old: null, new: null })
        } else if (dl.type === 'context') {
          lineNums.push({ old: oldLine, new: newLine })
          oldLine++; newLine++
        } else if (dl.type === 'added') {
          lineNums.push({ old: null, new: newLine })
          newLine++
        } else {
          lineNums.push({ old: oldLine, new: null })
          oldLine++
        }
      }

      const rawText = allLines.slice(start, end).join('\n') + '\n'
      hunks.push({
        header: allLines[start],
        lines: hunkLines,
        lineNums,
        rawText,
      })
    }

    if (hunks.length > 0) {
      result.push({ fileName, diffPrefix, hunks })
    }
  }

  return result
})

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

/** Build a full patch string for one hunk（使用 git 原始 hunk 文本） */
function hunkPatch(section: FileDiffSection, hunk: HunkInfo): string {
  return `${section.diffPrefix}\n${hunk.rawText}`
}

/**
 * Build one filtered hunk containing only selected lines.
 * - Selected `+` lines -> kept as added
 * - Selected `-` lines -> kept as removed
 * - Non-selected `-` lines -> converted to context (no-op for staging)
 * - Non-selected `+` lines -> omitted entirely
 * - `@@` header line counts recalculated to match
 */
/**
 * 构造只包含 selectedIds 所选行的单个 hunk patch 文本。
 *
 * forRevert=false（staging，正向 apply --cached）：
 *   - 未选中 `+` → 省略（不 stage）
 *   - 未选中 `-` → 转为 context（行存在于 index，不动它）
 *
 * forRevert=true（revert，反向 apply -R 作用于工作区）：
 *   - 未选中 `+` → 转为 context（行存在于工作区，不动它）
 *   - 未选中 `-` → 省略（行不存在于工作区，若转成 context 则 git apply -R 找不到它）
 */
function buildFilteredHunk(hunk: HunkInfo, selectedIds: Set<string>, forRevert = false): string | null {
  const contentLines = hunk.lines.slice(1)  // skip the @@ header

  const headerText = hunk.lines[0].content
  const headerMatch = headerText.match(/^@@ -(\d+)(?:,(\d+))? \+(\d+)(?:,(\d+))? @@/)
  if (!headerMatch) return hunk.lines.map(l => l.content).join('\n') + '\n'

  const origStart = parseInt(headerMatch[1])
  const newStart = parseInt(headerMatch[3])

  // Preserve the optional context suffix (e.g., " func() {")
  const secondAt = headerText.indexOf('@@', headerText.indexOf('@@') + 2)
  const headerSuffix = secondAt >= 0 ? headerText.slice(secondAt + 2).trim() : ''

  const keptLines: string[] = []
  let origCount = 0
  let newCount = 0
  let skipNextNoNewline = false

  for (const line of contentLines) {
    if (line.type === 'noNewline') {
      if (!skipNextNoNewline) keptLines.push(line.content)
      skipNextNoNewline = false
      continue
    }

    if (line.type === 'added' && !selectedIds.has(line.id)) {
      if (forRevert) {
        // 未选中 + 存在于工作区，保留为 context，不被 revert
        keptLines.push(' ' + line.content.slice(1))
        origCount++
        newCount++
        skipNextNoNewline = false
      } else {
        skipNextNoNewline = true
      }
      continue
    }
    skipNextNoNewline = false

    if (line.type === 'context') {
      keptLines.push(line.content)
      origCount++
      newCount++
    } else if (line.type === 'added') {
      keptLines.push(line.content)
      newCount++
    } else if (line.type === 'removed') {
      if (selectedIds.has(line.id)) {
        keptLines.push(line.content)
        origCount++
      } else if (forRevert) {
        // 未选中 - 不存在于工作区，省略（转成 context 会导致 git apply -R 找不到该行）
        skipNextNoNewline = true
      } else {
        // staging：未选中 - 存在于 index，转为 context 保留
        keptLines.push(' ' + line.content.slice(1))
        origCount++
        newCount++
      }
    }
  }

  const hasWork = contentLines.some(l => (l.type === 'added' || l.type === 'removed') && selectedIds.has(l.id))
  if (!hasWork) return null

  const newHeader = `@@ -${origStart},${origCount} +${newStart},${newCount} @@${headerSuffix ? ' ' + headerSuffix : ''}`
  return [newHeader, ...keptLines].join('\n') + '\n'
}

function buildFilteredPatch(section: FileDiffSection, hunk: HunkInfo, selectedIds: Set<string>, forRevert = false): string | null {
  const body = buildFilteredHunk(hunk, selectedIds, forRevert)
  return body ? `${section.diffPrefix}\n${body}` : null
}

/** Stage the entire file */
function stageEntireFile(path: string) {
  emit('stageFile', path)
}

function revertEntireFile(path: string) {
  emit('revertFile', path, props.workspaceIsStaged)
}

/** Stage one hunk */
function stageHunk(sectionIdx: number, hunkIdx: number) {
  const section = sections.value[sectionIdx]
  const hunk = section.hunks[hunkIdx]
  const patch = hunkPatch(section, hunk)
  emit('stagePatch', patch)
}

function revertHunk(sectionIdx: number, hunkIdx: number) {
  const section = sections.value[sectionIdx]
  const hunk = section.hunks[hunkIdx]
  const patch = hunkPatch(section, hunk)
  emit('revertPatch', patch, props.workspaceIsStaged)
}

/** Stage the hunks containing selected lines (only selected lines within each) */
function stageSelectedLines() {
  const ids = selectedLineIds.value
  if (ids.size === 0) return

  const patchParts: string[] = []
  for (const section of sections.value) {
    const hunkBodies: string[] = []
    for (const hunk of section.hunks) {
      const hasSelected = hunk.lines.some(l => ids.has(l.id))
      if (!hasSelected) continue

      const body = buildFilteredHunk(hunk, ids)
      if (body) hunkBodies.push(body)
    }

    if (hunkBodies.length > 0) {
      patchParts.push(`${section.diffPrefix}\n${hunkBodies.join('')}`)
    }
  }

  if (patchParts.length > 0) {
    emit('stagePatch', patchParts.join(''))
  }

  selectedLineIds.value = new Set()
  anchorLineId.value = null
}

function revertSelectedLines() {
  const ids = selectedLineIds.value
  if (ids.size === 0) return

  const patchParts: string[] = []
  for (const section of sections.value) {
    const hunkBodies: string[] = []
    for (const hunk of section.hunks) {
      const hasSelected = hunk.lines.some(l => ids.has(l.id))
      if (!hasSelected) continue

      const body = buildFilteredHunk(hunk, ids, true)
      if (body) hunkBodies.push(body)
    }

    if (hunkBodies.length > 0) {
      patchParts.push(`${section.diffPrefix}\n${hunkBodies.join('')}`)
    }
  }

  if (patchParts.length > 0) {
    emit('revertPatch', patchParts.join(''), props.workspaceIsStaged)
  }

  selectedLineIds.value = new Set()
  anchorLineId.value = null
}

// ── Line selection + change blocks ──

const selectableFlat = computed(() => {
  const out: { id: string; si: number; hi: number; li: number }[] = []
  for (let si = 0; si < sections.value.length; si++) {
    const section = sections.value[si]
    for (let hi = 0; hi < section.hunks.length; hi++) {
      const hunk = section.hunks[hi]
      for (let li = 0; li < hunk.lines.length; li++) {
        const line = hunk.lines[li]
        if (line.type !== 'header' && line.type !== 'noNewline') out.push({ id: line.id, si, hi, li })
      }
    }
  }
  return out
})

function flatIndexOf(id: string): number {
  return selectableFlat.value.findIndex((x) => x.id === id)
}

/** Hunk body (after @@): alternate runs of pure +/- vs context; only +/- runs get a stage block. */
type HunkBodySegment =
  | { kind: 'changes'; startLi: number; endLi: number }
  | { kind: 'context'; startLi: number; endLi: number }

function getHunkBodySegments(hunk: HunkInfo): HunkBodySegment[] {
  const L = hunk.lines
  const n = L.length
  if (n <= 1) return []
  const out: HunkBodySegment[] = []
  let li = 1
  while (li < n) {
    const t = L[li].type
    if (t === 'added' || t === 'removed') {
      const start = li
      while (li < n && (L[li].type === 'added' || L[li].type === 'removed')) li++
      while (li < n && L[li].type === 'noNewline') li++
      out.push({ kind: 'changes', startLi: start, endLi: li - 1 })
    } else {
      const start = li
      while (li < n && (L[li].type === 'context' || L[li].type === 'noNewline')) li++
      out.push({ kind: 'context', startLi: start, endLi: li - 1 })
    }
  }
  return out
}

/** 旧名兼容（缓存 / HMR 仍可能调用）；等价于仅返回连续 +/- 段。 */
function getChangeBlocksOrFull(hunk: HunkInfo): { startLi: number; endLi: number }[] {
  return getHunkBodySegments(hunk).filter(
    (s): s is { kind: 'changes'; startLi: number; endLi: number } => s.kind === 'changes',
  )
}

function changeLineIdsInBlock(hunk: HunkInfo, blk: { startLi: number; endLi: number }): Set<string> {
  const ids = new Set<string>()
  for (let li = blk.startLi; li <= blk.endLi; li++) {
    const line = hunk.lines[li]
    if (!line) continue
    if (line.type === 'added' || line.type === 'removed') ids.add(line.id)
  }
  return ids
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

function stageChangeBlock(sectionIdx: number, hunkIdx: number, blk: { startLi: number; endLi: number }) {
  const section = sections.value[sectionIdx]
  const hunk = section.hunks[hunkIdx]
  const ids = changeLineIdsInBlock(hunk, blk)
  if (ids.size === 0) return
  const patch = buildFilteredPatch(section, hunk, ids)
  if (patch) emit('stagePatch', patch)
}

function revertChangeBlock(sectionIdx: number, hunkIdx: number, blk: { startLi: number; endLi: number }) {
  const section = sections.value[sectionIdx]
  const hunk = section.hunks[hunkIdx]
  const ids = changeLineIdsInBlock(hunk, blk)
  if (ids.size === 0) return
  const patch = buildFilteredPatch(section, hunk, ids, true)
  if (patch) emit('revertPatch', patch, props.workspaceIsStaged)
}

function onGutterMouseDown(e: MouseEvent, lineId: string) {
  e.stopPropagation()
  const next = new Set<string>()
  const flat = selectableFlat.value
  const cur = flatIndexOf(lineId)
  if (cur < 0) return

  if (e.shiftKey && anchorLineId.value != null) {
    const a = flatIndexOf(anchorLineId.value)
    if (a >= 0) {
      const lo = Math.min(a, cur)
      const hi = Math.max(a, cur)
      for (let k = lo; k <= hi; k++) next.add(flat[k].id)
      selectedLineIds.value = next
      return
    }
  }

  if (e.ctrlKey || e.metaKey) {
    for (const id of selectedLineIds.value) next.add(id)
    if (next.has(lineId)) next.delete(lineId)
    else next.add(lineId)
    selectedLineIds.value = next
    anchorLineId.value = lineId
    return
  }

  next.add(lineId)
  selectedLineIds.value = next
  anchorLineId.value = lineId
}

function clearLineSelection() {
  selectedLineIds.value = new Set()
  anchorLineId.value = null
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

onMounted(() => window.addEventListener('keydown', onGlobalKeyDown))
onUnmounted(() => window.removeEventListener('keydown', onGlobalKeyDown))

</script>

<template>
  <div class="h-full flex flex-col bg-[--bg-primary]">
    <!-- Diff header bar -->
    <div class="px-2.5 py-2.5 text-xs text-[--text-secondary] border-b border-[--border-color] flex items-center gap-2 flex-shrink-0 min-w-0">
      <span v-if="fileName" class="text-[--text-primary] font-medium truncate font-mono-ui min-w-0 flex-1">{{ fileName }}</span>
      <span v-else class="text-[--text-secondary] shrink-0">选择文件查看差异</span>
      <span v-if="sections.length > 1" class="text-[10px] text-[--text-secondary] font-mono-ui shrink-0">{{ sections.length }} 个文件</span>
      <div
        v-if="(showHunkRevert || canStage) && selectedLineIds.size > 0"
        class="ml-auto flex shrink-0 items-center gap-1.5"
      >
        <button
          v-if="showHunkRevert"
          :disabled="patchStaging"
          class="flex items-center gap-1.5 px-2.5 py-2.5 rounded-[var(--radius)] text-[10px] bg-orange-700 text-white hover:bg-orange-600 transition-colors cursor-pointer disabled:opacity-40 disabled:cursor-not-allowed"
          @click="revertSelectedLines"
        >
          <Undo2 :size="12" />
          Revert 选中 ({{ selectedLineIds.size }})
        </button>
        <button
          v-if="canStage"
          :disabled="patchStaging"
          class="flex items-center gap-1.5 px-2.5 py-2.5 rounded-[var(--radius)] text-[10px] bg-green-700 text-white hover:bg-green-600 transition-colors cursor-pointer disabled:opacity-40 disabled:cursor-not-allowed"
          @click="stageSelectedLines"
        >
          <FilePlus :size="12" />
          Stage 选中 ({{ selectedLineIds.size }})
        </button>
      </div>
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
                <div class="ml-2 flex shrink-0 items-center gap-1 opacity-0 group-hover:opacity-100">
                  <button
                    v-if="showHunkRevert"
                    :disabled="patchStaging"
                    class="flex items-center gap-1 px-2.5 py-2.5 rounded-[var(--radius)] text-[10px] bg-orange-800/50 hover:bg-orange-700 text-orange-200 transition-colors cursor-pointer disabled:opacity-40 disabled:cursor-not-allowed"
                    title="丢弃整个 @@ 块变更"
                    @click.stop="revertHunk(si, hi)"
                  >
                    <Undo2 :size="12" />
                    Revert 块
                  </button>
                  <button
                    v-if="canStage"
                    :disabled="patchStaging"
                    class="flex items-center gap-1 px-2.5 py-2.5 rounded-[var(--radius)] text-[10px] bg-green-800/50 hover:bg-green-700 text-green-300 transition-colors cursor-pointer disabled:opacity-40 disabled:cursor-not-allowed"
                    title="Stage 整个 @@ 块（含全部子区域）"
                    @click.stop="stageHunk(si, hi)"
                  >
                    <FilePlus :size="12" />
                    Stage 块
                  </button>
                </div>
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
                      v-if="(showHunkRevert || canStage) && changeLineIdsInBlock(hunk, seg).size > 0"
                      class="absolute right-2.5 top-2.5 z-40 flex items-center gap-1 opacity-0 pointer-events-none group-hover/diffblk:opacity-100 group-hover/diffblk:pointer-events-auto transition-opacity"
                    >
                      <button
                        v-if="showHunkRevert"
                        :disabled="patchStaging"
                        type="button"
                        class="diff-revert-float flex items-center gap-1 px-2 py-1 rounded-[var(--radius)] text-[10px] bg-orange-700 text-white shadow-md cursor-pointer disabled:opacity-40 disabled:cursor-not-allowed"
                        title="丢弃本段连续 +/- 行"
                        @mousedown.stop
                        @click.stop="revertChangeBlock(si, hi, seg)"
                      >
                        <Undo2 :size="12" />
                        Revert
                      </button>
                      <button
                        v-if="canStage"
                        :disabled="patchStaging"
                        type="button"
                        class="diff-stage-float flex items-center gap-1 px-2 py-1 rounded-[var(--radius)] text-[10px] bg-green-700 text-white shadow-md cursor-pointer disabled:opacity-40 disabled:cursor-not-allowed"
                        title="Stage 本段连续 +/- 行"
                        @mousedown.stop
                        @click.stop="stageChangeBlock(si, hi, seg)"
                      >
                        <FilePlus :size="12" />
                        Stage
                      </button>
                    </div>
                    <div
                      v-for="li in liRange(seg.startLi, seg.endLi, hunk.lines.length - 1)"
                      :key="`${si}-${hi}-${li}-${hunk.lines[li]?.id ?? 'x'}`"
                      class="flex transition-colors"
                      :class="{
                        'bg-[--diff-added] hover:bg-green-800/40': hunk.lines[li]?.type === 'added',
                        'bg-[--diff-removed] hover:bg-red-800/40': hunk.lines[li]?.type === 'removed',
                        'hover:bg-[--bg-tertiary]': hunk.lines[li]?.type === 'context',
                        'text-[--text-secondary]': hunk.lines[li]?.type === 'header',
                        'text-[--text-primary]': hunk.lines[li]?.type === 'context',
                        'text-[--diff-added-text]': hunk.lines[li]?.type === 'added',
                        'text-[--diff-removed-text]': hunk.lines[li]?.type === 'removed',
                        'outline outline-1 outline-green-500/60 -outline-offset-1': hunk.lines[li] && selectedLineIds.has(hunk.lines[li].id),
                      }"
                    >
                      <div
                        v-if="hunk.lines[li]?.type !== 'header'"
                        class="diff-select-zone flex flex-shrink-0 cursor-pointer select-none"
                        @mousedown="hunk.lines[li] && onGutterMouseDown($event, hunk.lines[li].id)"
                      >
                        <div
                          class="flex-shrink-0 w-3 flex items-center justify-center text-[10px]"
                          :class="hunk.lines[li] && selectedLineIds.has(hunk.lines[li].id) ? 'text-green-400' : 'text-transparent'"
                        >{{ hunk.lines[li] && selectedLineIds.has(hunk.lines[li].id) ? '✓' : '○' }}</div>
                        <div class="flex-shrink-0 w-7 text-right pr-1 text-[--text-secondary]/35">{{ hunk.lineNums[li]?.old ?? '' }}</div>
                        <div class="flex-shrink-0 w-7 text-right pr-2.5 text-[--text-secondary]/50">{{ hunk.lineNums[li]?.new ?? '' }}</div>
                      </div>
                      <template v-else>
                        <div class="flex-shrink-0 w-3" />
                        <div class="flex-shrink-0 w-7 pr-1 select-none" />
                        <div class="flex-shrink-0 w-7 pr-2.5 select-none" />
                      </template>
                      <span class="px-2.5 whitespace-pre-wrap flex-1 min-w-0 select-text">{{ hunk.lines[li]?.content }}</span>
                    </div>
                    <!-- 叠在行上方：不挡点击/划选；hover 整块时可见描边 + 淡绿罩 -->
                    <div
                      aria-hidden="true"
                      class="pointer-events-none absolute inset-0 z-[1] rounded-sm border-2 border-transparent opacity-0 transition-[opacity,border-color,background-color] duration-150 group-hover/diffblk:border-green-500/70 group-hover/diffblk:bg-green-500/15 group-hover/diffblk:opacity-100"
                    />
                  </div>
                  <template v-else>
                    <div
                      v-for="li in liRange(seg.startLi, seg.endLi, hunk.lines.length - 1)"
                      :key="`${si}-${hi}-${li}-${hunk.lines[li]?.id ?? 'x'}`"
                      class="flex transition-colors"
                      :class="{
                        'bg-[--diff-added] hover:bg-green-800/40': hunk.lines[li]?.type === 'added',
                        'bg-[--diff-removed] hover:bg-red-800/40': hunk.lines[li]?.type === 'removed',
                        'hover:bg-[--bg-tertiary]': hunk.lines[li]?.type === 'context',
                        'text-[--text-secondary]': hunk.lines[li]?.type === 'header',
                        'text-[--text-primary]': hunk.lines[li]?.type === 'context',
                        'text-[--diff-added-text]': hunk.lines[li]?.type === 'added',
                        'text-[--diff-removed-text]': hunk.lines[li]?.type === 'removed',
                        'outline outline-1 outline-green-500/60 -outline-offset-1': hunk.lines[li] && selectedLineIds.has(hunk.lines[li].id),
                      }"
                    >
                      <div
                        v-if="hunk.lines[li]?.type !== 'header'"
                        class="diff-select-zone flex flex-shrink-0 cursor-pointer select-none"
                        @mousedown="hunk.lines[li] && onGutterMouseDown($event, hunk.lines[li].id)"
                      >
                        <div
                          class="flex-shrink-0 w-3 flex items-center justify-center text-[10px]"
                          :class="hunk.lines[li] && selectedLineIds.has(hunk.lines[li].id) ? 'text-green-400' : 'text-transparent'"
                        >{{ hunk.lines[li] && selectedLineIds.has(hunk.lines[li].id) ? '✓' : '○' }}</div>
                        <div class="flex-shrink-0 w-7 text-right pr-1 text-[--text-secondary]/35">{{ hunk.lineNums[li]?.old ?? '' }}</div>
                        <div class="flex-shrink-0 w-7 text-right pr-2.5 text-[--text-secondary]/50">{{ hunk.lineNums[li]?.new ?? '' }}</div>
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
