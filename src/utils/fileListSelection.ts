export interface FileListSelectionClickInput {
  sectionPaths: string[]
  current: Set<string>
  clickedPath: string
  anchorPath: string | null
  shiftKey: boolean
  toggleKey: boolean
}

export interface FileListSelectionResult {
  selected: Set<string>
  anchorPath: string | null
}

function pathIndex(paths: string[], path: string): number {
  return paths.indexOf(path)
}

export function fileRangeSelection(paths: string[], startPath: string, endPath: string): Set<string> {
  const start = pathIndex(paths, startPath)
  const end = pathIndex(paths, endPath)
  const selected = new Set<string>()
  if (start < 0 || end < 0) return selected

  const lo = Math.min(start, end)
  const hi = Math.max(start, end)
  for (let index = lo; index <= hi; index++) selected.add(paths[index])
  return selected
}

export function applyFileListSelectionClick(input: FileListSelectionClickInput): FileListSelectionResult {
  const { sectionPaths, clickedPath } = input
  if (pathIndex(sectionPaths, clickedPath) < 0) {
    return { selected: new Set(input.current), anchorPath: input.anchorPath }
  }

  const sectionSet = new Set(sectionPaths)
  const currentInSection = new Set([...input.current].filter((path) => sectionSet.has(path)))

  if (input.shiftKey && input.anchorPath != null && sectionSet.has(input.anchorPath)) {
    const selected = fileRangeSelection(sectionPaths, input.anchorPath, clickedPath)
    if (selected.size > 0) return { selected, anchorPath: input.anchorPath }
  }

  if (input.toggleKey) {
    const selected = new Set(currentInSection)
    if (selected.has(clickedPath)) selected.delete(clickedPath)
    else selected.add(clickedPath)
    return { selected, anchorPath: clickedPath }
  }

  return { selected: new Set([clickedPath]), anchorPath: clickedPath }
}
