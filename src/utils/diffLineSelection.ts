export interface SelectableLineRef {
  id: string
}

export interface LineSelectionClickInput {
  lines: SelectableLineRef[]
  current: Set<string>
  clickedId: string
  anchorId: string | null
  shiftKey: boolean
  toggleKey: boolean
}

export interface LineSelectionResult {
  selected: Set<string>
  anchorId: string | null
}

function lineIndex(lines: SelectableLineRef[], id: string): number {
  return lines.findIndex((line) => line.id === id)
}

export function lineRangeSelection(lines: SelectableLineRef[], startId: string, endId: string): Set<string> {
  const start = lineIndex(lines, startId)
  const end = lineIndex(lines, endId)
  const selected = new Set<string>()
  if (start < 0 || end < 0) return selected

  const lo = Math.min(start, end)
  const hi = Math.max(start, end)
  for (let index = lo; index <= hi; index++) selected.add(lines[index].id)
  return selected
}

export function applyLineSelectionClick(input: LineSelectionClickInput): LineSelectionResult {
  const clickedIndex = lineIndex(input.lines, input.clickedId)
  if (clickedIndex < 0) {
    return { selected: new Set(input.current), anchorId: input.anchorId }
  }

  if (input.shiftKey && input.anchorId != null) {
    const selected = lineRangeSelection(input.lines, input.anchorId, input.clickedId)
    if (selected.size > 0) return { selected, anchorId: input.anchorId }
  }

  if (input.toggleKey) {
    const selected = new Set(input.current)
    if (selected.has(input.clickedId)) selected.delete(input.clickedId)
    else selected.add(input.clickedId)
    return { selected, anchorId: input.clickedId }
  }

  return { selected: new Set([input.clickedId]), anchorId: input.clickedId }
}
