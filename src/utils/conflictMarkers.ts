export type ConflictResolution = 'ours' | 'theirs' | 'both'

export interface ConflictBlock {
  index: number
  startLine: number
  separatorLine: number
  endLine: number
  ours: string[]
  theirs: string[]
}

function splitLines(content: string): { lines: string[]; trailingNewline: boolean } {
  const trailingNewline = content.endsWith('\n')
  const lines = content.split('\n')
  if (trailingNewline) lines.pop()
  return { lines, trailingNewline }
}

function joinLines(lines: string[], trailingNewline: boolean): string {
  return `${lines.join('\n')}${trailingNewline ? '\n' : ''}`
}

export function parseConflictBlocks(content: string): ConflictBlock[] {
  const { lines } = splitLines(content)
  const blocks: ConflictBlock[] = []
  let lineIndex = 0

  while (lineIndex < lines.length) {
    if (!lines[lineIndex].startsWith('<<<<<<<')) {
      lineIndex++
      continue
    }

    const startLine = lineIndex
    const ours: string[] = []
    const theirs: string[] = []
    lineIndex++

    while (lineIndex < lines.length && !lines[lineIndex].startsWith('=======')) {
      ours.push(lines[lineIndex])
      lineIndex++
    }

    if (lineIndex >= lines.length) break
    const separatorLine = lineIndex
    lineIndex++

    while (lineIndex < lines.length && !lines[lineIndex].startsWith('>>>>>>>')) {
      theirs.push(lines[lineIndex])
      lineIndex++
    }

    if (lineIndex >= lines.length) break
    const endLine = lineIndex

    blocks.push({
      index: blocks.length,
      startLine,
      separatorLine,
      endLine,
      ours,
      theirs,
    })
    lineIndex++
  }

  return blocks
}

export function resolveConflictBlock(
  content: string,
  blockIndex: number,
  resolution: ConflictResolution,
): string {
  const { lines, trailingNewline } = splitLines(content)
  const block = parseConflictBlocks(content).find((item) => item.index === blockIndex)
  if (!block) return content

  const replacement =
    resolution === 'ours'
      ? block.ours
      : resolution === 'theirs'
        ? block.theirs
        : [...block.ours, ...block.theirs]

  lines.splice(block.startLine, block.endLine - block.startLine + 1, ...replacement)
  return joinLines(lines, trailingNewline)
}
