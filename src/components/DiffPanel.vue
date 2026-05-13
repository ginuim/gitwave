<script setup lang="ts">
import { ref, computed } from 'vue'
import { ChevronDown, ChevronRight, FileCode, FilePlus, User, CalendarDays } from 'lucide-vue-next'

const props = defineProps<{
  diffText: string
  fileName: string | null
  canStage: boolean
  filePath: string | null
}>()

const emit = defineEmits<{
  stageFile: [path: string]
  stagePatch: [patch: string]
}>()

interface DiffLine {
  type: 'header' | 'added' | 'removed' | 'context'
  content: string
  /** Unique id for line-selection tracking */
  id: string
}

interface HunkInfo {
  header: string   // e.g. "@@ -1,5 +1,7 @@"
  lines: DiffLine[]
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

// Line selection: set of line ids
const selectedLineIds = ref<Set<string>>(new Set())

function toggleLine(lineId: string) {
  const next = new Set(selectedLineIds.value)
  if (next.has(lineId)) {
    next.delete(lineId)
  } else {
    next.add(lineId)
  }
  selectedLineIds.value = next
}

// ── Commit info ──

interface CommitInfo {
  hash: string
  author: string
  date: string
  message: string
}

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

    // Build the diff prefix (without index line for apply compatibility)
    const diffGitLine = allLines[0]
    // Find --- a/ and +++ b/ lines
    const minusLine = allLines.find(l => l.startsWith('--- ')) ?? `--- a/${fileName}`
    const plusLine = allLines.find(l => l.startsWith('+++ ')) ?? `+++ b/${fileName}`
    const diffPrefix = `${diffGitLine}\n${minusLine}\n${plusLine}`

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
        else type = 'context'

        hunkLines.push({ type, content: line, id: nextLineId() })
      }

      hunks.push({
        header: allLines[start],
        lines: hunkLines,
      })
    }

    if (hunks.length > 0) {
      result.push({ fileName, diffPrefix, hunks })
    }
  }

  return result
})

// ── Stage helpers ──

/** Build a full patch string for one hunk */
function hunkPatch(section: FileDiffSection, hunk: HunkInfo): string {
  const body = hunk.lines.map(l => l.content).join('\n')
  return `${section.diffPrefix}\n${body}\n`
}

/**
 * Build a filtered patch containing only selected lines within a hunk.
 * - Selected `+` lines → kept as added
 * - Selected `-` lines → kept as removed
 * - Non-selected `-` lines → converted to context (no-op for staging)
 * - Non-selected `+` lines → omitted entirely
 * - `@@` header line counts recalculated to match
 */
function buildFilteredPatch(section: FileDiffSection, hunk: HunkInfo, selectedIds: Set<string>): string | null {
  const contentLines = hunk.lines.slice(1)  // skip the @@ header

  const headerText = hunk.lines[0].content
  const headerMatch = headerText.match(/^@@ -(\d+)(?:,(\d+))? \+(\d+)(?:,(\d+))? @@/)
  if (!headerMatch) return hunkPatch(section, hunk) // fallback

  const origStart = parseInt(headerMatch[1])
  const newStart = parseInt(headerMatch[3])

  // Preserve the optional context suffix (e.g., " func() {")
  const secondAt = headerText.indexOf('@@', headerText.indexOf('@@') + 2)
  const headerSuffix = secondAt >= 0 ? headerText.slice(secondAt + 2).trim() : ''

  const keptLines: string[] = []
  let origCount = 0
  let newCount = 0

  for (const line of contentLines) {
    if (line.type === 'context') {
      keptLines.push(line.content)
      origCount++
      newCount++
    } else if (line.type === 'added') {
      if (selectedIds.has(line.id)) {
        keptLines.push(line.content)
        newCount++
      }
      // Not selected: omit entirely
    } else if (line.type === 'removed') {
      if (selectedIds.has(line.id)) {
        keptLines.push(line.content)
        origCount++
      } else {
        // Convert to context (keep the line in the file)
        keptLines.push(' ' + line.content.slice(1))
        origCount++
        newCount++
      }
    }
  }

  // Check if there's actually anything to stage
  const hasWork = contentLines.some(l => (l.type === 'added' || l.type === 'removed') && selectedIds.has(l.id))
  if (!hasWork) return null

  // Recalculate @@ line counts from the actual patch content
  // (Some context lines don't add/remove; removed lines count only in orig; added lines count only in new)
  const newHeader = `@@ -${origStart},${origCount} +${newStart},${newCount} @@${headerSuffix ? ' ' + headerSuffix : ''}`
  const body = [newHeader, ...keptLines].join('\n')
  return `${section.diffPrefix}\n${body}\n`
}

/** Stage the entire file */
function stageEntireFile(path: string) {
  emit('stageFile', path)
}

/** Stage one hunk */
function stageHunk(sectionIdx: number, hunkIdx: number) {
  const section = sections.value[sectionIdx]
  const hunk = section.hunks[hunkIdx]
  const patch = hunkPatch(section, hunk)
  emit('stagePatch', patch)
}

/** Stage the hunks containing selected lines (only selected lines within each) */
function stageSelectedLines() {
  const ids = selectedLineIds.value
  if (ids.size === 0) return

  for (const section of sections.value) {
    for (const hunk of section.hunks) {
      const hasSelected = hunk.lines.some(l => ids.has(l.id))
      if (!hasSelected) continue

      const patch = buildFilteredPatch(section, hunk, ids)
      if (patch) {
        emit('stagePatch', patch)
      }
    }
  }

  selectedLineIds.value = new Set()
}


</script>

<template>
  <div class="h-full flex flex-col bg-[--bg-primary]">
    <!-- Diff header bar -->
    <div class="px-4 py-2 text-xs text-[--text-secondary] border-b border-[--border-color] flex items-center gap-2 flex-shrink-0">
      <span v-if="fileName" class="text-[--text-primary] font-medium truncate">{{ fileName }}</span>
      <span v-else class="text-[--text-secondary]">选择文件查看差异</span>
      <span v-if="sections.length > 1" class="ml-auto text-[--text-secondary]">{{ sections.length }} 个文件</span>
      <!-- Stage selected lines button -->
      <button
        v-if="canStage && selectedLineIds.size > 0"
        class="ml-2 flex items-center gap-1 px-2 py-0.5 rounded text-[10px] bg-green-700 text-white hover:bg-green-600 transition-colors"
        @click="stageSelectedLines"
      >
        <FilePlus :size="11" />
        暂存选中 ({{ selectedLineIds.size }})
      </button>
    </div>

    <!-- Diff content -->
    <div class="flex-1 overflow-y-auto">
      <div v-if="!diffText" class="p-4 text-[--text-secondary] text-sm">
        <template v-if="fileName">没有差异内容</template>
        <template v-else>点击左侧文件查看变更</template>
      </div>

      <template v-else>
        <!-- Commit info (from git show) -->
        <div v-if="commitInfo" class="px-4 py-3 border-b border-[--border-color] bg-[--bg-secondary]">
          <div class="flex items-center gap-2 mb-2">
            <span class="text-xs font-mono text-[--accent] font-medium">{{ commitInfo.hash }}</span>
          </div>
          <div class="flex items-center gap-4 text-[11px] text-[--text-secondary] mb-2">
            <span class="flex items-center gap-1">
              <User :size="12" />
              {{ commitInfo.author }}
            </span>
            <span class="flex items-center gap-1">
              <CalendarDays :size="12" />
              {{ commitInfo.date }}
            </span>
          </div>
          <div class="text-sm text-[--text-primary] font-medium leading-relaxed whitespace-pre-wrap">
            {{ commitInfo.message }}
          </div>
        </div>

        <!-- File diff sections -->
        <div
          v-for="(section, si) in sections"
          :key="si"
          class="border-b border-[--border-color]"
        >
          <!-- File header bar -->
          <div
            class="sticky top-0 z-10 flex items-center gap-2 px-4 py-1.5 bg-[--bg-tertiary] border-b border-[--border-color] cursor-pointer select-none"
            @click="toggleSection(si)"
          >
            <ChevronDown v-if="!isSectionCollapsed(si)" :size="14" class="flex-shrink-0 text-[--text-secondary]" />
            <ChevronRight v-else :size="14" class="flex-shrink-0 text-[--text-secondary]" />
            <FileCode :size="13" class="flex-shrink-0 text-[--text-secondary]" />
            <span class="text-xs text-[--text-primary] font-medium truncate">{{ section.fileName || '差异' }}</span>
            <div class="ml-auto flex items-center gap-2">
              <button
                v-if="canStage"
                class="flex items-center gap-0.5 px-1.5 py-0.5 rounded text-[10px] bg-green-700/70 hover:bg-green-600 text-white transition-colors"
                title="暂存整个文件"
                @click.stop="stageEntireFile(filePath || section.fileName)"
              >
                <FilePlus :size="11" />
                暂存文件
              </button>
              <span class="text-[10px] text-[--diff-added-text]">{{ section.hunks.reduce((s, h) => s + h.lines.filter(l => l.type === 'added').length, 0) }} +</span>
              <span class="text-[10px] text-[--diff-removed-text]">{{ section.hunks.reduce((s, h) => s + h.lines.filter(l => l.type === 'removed').length, 0) }} -</span>
            </div>
          </div>

          <!-- Hunks -->
          <div v-if="!isSectionCollapsed(si)">
            <div
              v-for="(hunk, hi) in section.hunks"
              :key="hi"
            >
              <!-- Hunk header -->
              <div
                class="sticky top-[30px] z-[5] flex items-center gap-2 px-4 py-1 bg-[--bg-secondary] border-b border-[--border-color] cursor-pointer select-none group"
                :class="{ 'opacity-60': isHunkCollapsed(si, hi) }"
                @click="toggleHunk(si, hi)"
              >
                <ChevronDown v-if="!isHunkCollapsed(si, hi)" :size="12" class="flex-shrink-0 text-[--text-secondary]" />
                <ChevronRight v-else :size="12" class="flex-shrink-0 text-[--text-secondary]" />
                <span class="text-[11px] font-mono text-[--text-secondary] truncate">{{ hunk.header }}</span>
                <button
                  v-if="canStage"
                  class="ml-auto flex items-center gap-0.5 px-1.5 py-0.5 rounded text-[10px] bg-green-800/50 hover:bg-green-700 text-green-300 transition-colors opacity-0 group-hover:opacity-100"
                  title="暂存此修改块"
                  @click.stop="stageHunk(si, hi)"
                >
                  <FilePlus :size="11" />
                  暂存块
                </button>
              </div>

              <!-- Hunk lines -->
              <div v-if="!isHunkCollapsed(si, hi)" class="font-mono text-xs leading-relaxed border-b border-[--border-color]">
                <div
                  v-for="(line, li) in hunk.lines"
                  :key="line.id"
                  class="flex cursor-pointer transition-colors"
                  :class="{
                    'bg-[--diff-added] hover:bg-green-800/40': line.type === 'added',
                    'bg-[--diff-removed] hover:bg-red-800/40': line.type === 'removed',
                    'hover:bg-[--bg-tertiary]': line.type === 'context',
                    'text-[--text-secondary]': line.type === 'header',
                    'text-[--text-primary]': line.type === 'context',
                    'text-[--diff-added-text]': line.type === 'added',
                    'text-[--diff-removed-text]': line.type === 'removed',
                    'outline outline-1 outline-green-500/60 -outline-offset-1': canStage && selectedLineIds.has(line.id),
                  }"
                  @click="canStage && toggleLine(line.id)"
                >
                  <!-- Selection indicator -->
                  <div
                    v-if="canStage && line.type !== 'header'"
                    class="flex-shrink-0 w-4 flex items-center justify-center text-[10px]"
                    :class="selectedLineIds.has(line.id) ? 'text-green-400' : 'text-transparent'"
                  >{{ selectedLineIds.has(line.id) ? '✓' : '○' }}</div>
                  <!-- Line number gutter -->
                  <div class="flex-shrink-0 w-8 text-right pr-2 select-none text-[--text-secondary]/50">{{ li + 1 }}</div>
                  <!-- Line content -->
                  <span class="px-4 whitespace-pre-wrap flex-1 min-w-0">{{ line.content }}</span>
                </div>
              </div>
            </div>
          </div>
        </div>
      </template>
    </div>
  </div>
</template>
