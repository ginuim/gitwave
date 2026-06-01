export type DiffLineType = 'header' | 'added' | 'removed' | 'context' | 'noNewline'
export type PatchMode = 'stage' | 'revert'

export interface DiffLine {
  key: string
  type: DiffLineType
  content: string
  oldLineNumber: number | null
  newLineNumber: number | null
}

export interface DiffHunk {
  key: string
  header: string
  lines: DiffLine[]
  rawText: string
}

export interface FileDiffSection {
  key: string
  fileName: string
  diffPrefix: string
  hunks: DiffHunk[]
}

export type HunkBodySegment =
  | { kind: 'changes'; startLineIndex: number; endLineIndex: number }
  | { kind: 'context'; startLineIndex: number; endLineIndex: number }

function lineKey(sectionIndex: number, hunkIndex: number, lineIndex: number): string {
  return `${sectionIndex}:${hunkIndex}:${lineIndex}`
}

function lineType(line: string): DiffLineType {
  if (line.startsWith('@@')) return 'header'
  if (line.startsWith('+')) return 'added'
  if (line.startsWith('-')) return 'removed'
  if (line.startsWith('\\')) return 'noNewline'
  return 'context'
}

function parseHunkHeader(header: string): { oldStart: number; newStart: number } | null {
  const match = header.match(/^@@ -(\d+)(?:,\d+)? \+(\d+)(?:,\d+)? @@/)
  if (!match) return null
  return {
    oldStart: Number.parseInt(match[1], 10),
    newStart: Number.parseInt(match[2], 10),
  }
}

function hunkHeaderSuffix(header: string): string {
  const secondAt = header.indexOf('@@', header.indexOf('@@') + 2)
  return secondAt >= 0 ? header.slice(secondAt + 2).trim() : ''
}

export function parseDiffSections(diffText: string, fallbackFileName: string | null): FileDiffSection[] {
  if (!diffText) return []

  const diffStart = diffText.indexOf('\ndiff --git ')
  const diffContent = diffStart >= 0 ? diffText.slice(diffStart + 1) : diffText
  const rawParts = diffContent.split('\ndiff --git ')
  const sections: FileDiffSection[] = []

  for (let sectionIndex = 0; sectionIndex < rawParts.length; sectionIndex++) {
    const part = rawParts[sectionIndex]
    if (!part.trim()) continue

    const fullText = sectionIndex === 0 ? part : `diff --git ${part}`
    const allLines = fullText.split('\n')
    const diffGitLine = allLines[0]
    const match = diffGitLine.match(/diff --git a\/(.+) b\/(.+)/)
    const fileName = match ? (match[2] || match[1]) : (fallbackFileName || '')

    const minusIdx = allLines.findIndex((line) => line.startsWith('--- '))
    const plusIdx = allLines.findIndex((line) => line.startsWith('+++ '))
    const metaLines = minusIdx > 1 ? allLines.slice(1, minusIdx) : []
    const minusLine = minusIdx >= 0 ? allLines[minusIdx] : `--- a/${fileName}`
    const plusLine = plusIdx >= 0 ? allLines[plusIdx] : `+++ b/${fileName}`
    const diffPrefix = [diffGitLine, ...metaLines, minusLine, plusLine].join('\n')

    const hunkStartIndices: number[] = []
    for (let i = 0; i < allLines.length; i++) {
      if (allLines[i].startsWith('@@')) hunkStartIndices.push(i)
    }
    if (hunkStartIndices.length === 0) continue

    const hunks: DiffHunk[] = []
    for (let hunkIndex = 0; hunkIndex < hunkStartIndices.length; hunkIndex++) {
      const start = hunkStartIndices[hunkIndex]
      const rawEnd = hunkIndex + 1 < hunkStartIndices.length ? hunkStartIndices[hunkIndex + 1] : allLines.length
      const end = rawEnd === allLines.length && allLines[rawEnd - 1] === '' ? rawEnd - 1 : rawEnd
      const header = allLines[start]
      const parsedHeader = parseHunkHeader(header)
      let oldLine = parsedHeader?.oldStart ?? 1
      let newLine = parsedHeader?.newStart ?? 1

      const lines: DiffLine[] = []
      for (let lineIndex = start; lineIndex < end; lineIndex++) {
        const content = allLines[lineIndex]
        const type = lineType(content)
        let oldLineNumber: number | null = null
        let newLineNumber: number | null = null

        if (type === 'context') {
          oldLineNumber = oldLine
          newLineNumber = newLine
          oldLine++
          newLine++
        } else if (type === 'added') {
          newLineNumber = newLine
          newLine++
        } else if (type === 'removed') {
          oldLineNumber = oldLine
          oldLine++
        }

        lines.push({
          key: lineKey(sections.length, hunkIndex, lineIndex - start),
          type,
          content,
          oldLineNumber,
          newLineNumber,
        })
      }

      hunks.push({
        key: `${sections.length}:${hunkIndex}`,
        header,
        lines,
        rawText: `${allLines.slice(start, end).join('\n')}\n`,
      })
    }

    sections.push({
      key: String(sections.length),
      fileName,
      diffPrefix,
      hunks,
    })
  }

  return sections
}

export function hunkPatch(section: FileDiffSection, hunk: DiffHunk): string {
  return `${section.diffPrefix}\n${hunk.rawText}`
}

export function getHunkBodySegments(hunk: DiffHunk): HunkBodySegment[] {
  const segments: HunkBodySegment[] = []
  let lineIndex = 1

  while (lineIndex < hunk.lines.length) {
    const type = hunk.lines[lineIndex].type
    if (type === 'added' || type === 'removed') {
      const startLineIndex = lineIndex
      while (
        lineIndex < hunk.lines.length &&
        (hunk.lines[lineIndex].type === 'added' || hunk.lines[lineIndex].type === 'removed')
      ) {
        lineIndex++
      }
      while (lineIndex < hunk.lines.length && hunk.lines[lineIndex].type === 'noNewline') lineIndex++
      segments.push({ kind: 'changes', startLineIndex, endLineIndex: lineIndex - 1 })
    } else {
      const startLineIndex = lineIndex
      while (
        lineIndex < hunk.lines.length &&
        (hunk.lines[lineIndex].type === 'context' || hunk.lines[lineIndex].type === 'noNewline')
      ) {
        lineIndex++
      }
      segments.push({ kind: 'context', startLineIndex, endLineIndex: lineIndex - 1 })
    }
  }

  return segments
}

export function lineKeysInChangeSegment(hunk: DiffHunk, segment: HunkBodySegment): Set<string> {
  const keys = new Set<string>()
  for (let lineIndex = segment.startLineIndex; lineIndex <= segment.endLineIndex; lineIndex++) {
    const line = hunk.lines[lineIndex]
    if (line?.type === 'added' || line?.type === 'removed') keys.add(line.key)
  }
  return keys
}

function buildFilteredHunk(hunk: DiffHunk, selectedKeys: Set<string>, mode: PatchMode): string | null {
  const headerInfo = parseHunkHeader(hunk.header)
  if (!headerInfo) return hunk.rawText

  const keptLines: string[] = []
  let oldCount = 0
  let newCount = 0
  let skipNextNoNewline = false
  let hasChange = false

  for (const line of hunk.lines.slice(1)) {
    if (line.type === 'noNewline') {
      if (!skipNextNoNewline) keptLines.push(line.content)
      skipNextNoNewline = false
      continue
    }

    const selected = selectedKeys.has(line.key)
    if ((line.type === 'added' || line.type === 'removed') && selected) {
      hasChange = true
    }

    if (line.type === 'context') {
      keptLines.push(line.content)
      oldCount++
      newCount++
      skipNextNoNewline = false
    } else if (line.type === 'added') {
      if (selected) {
        keptLines.push(line.content)
        newCount++
        skipNextNoNewline = false
      } else if (mode === 'revert') {
        keptLines.push(` ${line.content.slice(1)}`)
        oldCount++
        newCount++
        skipNextNoNewline = false
      } else {
        skipNextNoNewline = true
      }
    } else if (line.type === 'removed') {
      if (selected) {
        keptLines.push(line.content)
        oldCount++
        skipNextNoNewline = false
      } else if (mode === 'stage') {
        keptLines.push(` ${line.content.slice(1)}`)
        oldCount++
        newCount++
        skipNextNoNewline = false
      } else {
        skipNextNoNewline = true
      }
    }
  }

  if (!hasChange) return null

  const suffix = hunkHeaderSuffix(hunk.header)
  const header = `@@ -${headerInfo.oldStart},${oldCount} +${headerInfo.newStart},${newCount} @@${suffix ? ` ${suffix}` : ''}`
  return `${[header, ...keptLines].join('\n')}\n`
}

export function buildPatchForSelection(
  sections: FileDiffSection[],
  selectedKeys: Set<string>,
  mode: PatchMode,
): string | null {
  const patchParts: string[] = []

  for (const section of sections) {
    const hunkBodies: string[] = []
    for (const hunk of section.hunks) {
      const hunkHasSelection = hunk.lines.some((line) => selectedKeys.has(line.key))
      if (!hunkHasSelection) continue

      const body = buildFilteredHunk(hunk, selectedKeys, mode)
      if (body) hunkBodies.push(body)
    }
    if (hunkBodies.length > 0) patchParts.push(`${section.diffPrefix}\n${hunkBodies.join('')}`)
  }

  return patchParts.length > 0 ? patchParts.join('') : null
}

export function buildPatchForSegment(
  section: FileDiffSection,
  hunk: DiffHunk,
  segment: HunkBodySegment,
  mode: PatchMode,
): string | null {
  return buildPatchForSelection([section], lineKeysInChangeSegment(hunk, segment), mode)
}

function serializeHunkBody(lines: DiffLine[]): string | null {
  const headerLine = lines.find((line) => line.type === 'header')
  if (!headerLine) return null

  const headerInfo = parseHunkHeader(headerLine.content)
  const bodyLines: string[] = []
  let oldCount = 0
  let newCount = 0
  let hasChange = false

  for (const line of lines) {
    if (line.type === 'header') continue
    if (line.type === 'context') {
      oldCount++
      newCount++
      bodyLines.push(line.content)
    } else if (line.type === 'added') {
      newCount++
      hasChange = true
      bodyLines.push(line.content)
    } else if (line.type === 'removed') {
      oldCount++
      hasChange = true
      bodyLines.push(line.content)
    } else if (line.type === 'noNewline') {
      bodyLines.push(line.content)
    }
  }

  if (!hasChange || (oldCount === 0 && newCount === 0)) return null

  const suffix = hunkHeaderSuffix(headerLine.content)
  const oldStart = headerInfo?.oldStart ?? 1
  const newStart = headerInfo?.newStart ?? 1
  const header = `@@ -${oldStart},${oldCount} +${newStart},${newCount} @@${suffix ? ` ${suffix}` : ''}`
  return `${[header, ...bodyLines].join('\n')}\n`
}

/** revert 后乐观更新 diff 文本：移除将被丢弃的 +/- 行 */
export function applyOptimisticRevertToDiffText(
  diffText: string,
  fileName: string | null,
  revertedKeys: Set<string>,
): string {
  if (!diffText || revertedKeys.size === 0) return diffText

  const diffStart = diffText.indexOf('\ndiff --git ')
  const prefix = diffStart >= 0 ? diffText.slice(0, diffStart + 1) : ''
  const sections = parseDiffSections(diffText, fileName)
  const patchParts: string[] = []

  for (const section of sections) {
    const hunkTexts: string[] = []
    for (const hunk of section.hunks) {
      const remaining = hunk.lines.flatMap((line): DiffLine[] => {
        if (!revertedKeys.has(line.key)) return [line]
        if (line.type === 'added') return []
        if (line.type === 'removed') {
          return [{
            ...line,
            type: 'context',
            content: ` ${line.content.slice(1)}`,
          }]
        }
        return [line]
      })
      const body = serializeHunkBody(remaining)
      if (body) hunkTexts.push(body)
    }
    if (hunkTexts.length > 0) {
      patchParts.push(`${section.diffPrefix}\n${hunkTexts.join('')}`)
    }
  }

  const newDiff = patchParts.join('')
  if (!newDiff) {
    return prefix.trimEnd()
  }
  return prefix + newDiff
}
