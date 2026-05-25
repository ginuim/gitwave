import type { CommitLog } from '../types'

export const GRAPH_ROW_HEIGHT = 56
export const GRAPH_LANE_WIDTH = 16
export const GRAPH_NODE_RADIUS = 4
export const GRAPH_NODE_RADIUS_ACTIVE = 6

const MAIN_LANE_COLOR = '#94a3b8'
const BRANCH_LANE_COLORS = [
  '#3b82f6',
  '#f59e0b',
  '#22c55e',
  '#a855f7',
  '#ec4899',
  '#06b6d4',
]

export interface GraphNode {
  hash: string
  row: number
  column: number
  color: string
}

export interface GraphPath {
  d: string
  color: string
  fromHash: string
  toHash: string
  fromColumn: number
  toColumn: number
  fromRow: number
  toRow: number
}

export interface GraphLane {
  id: string
  column: number
  label: string
  color: string
}

export interface CommitGraphLayout {
  nodes: GraphNode[]
  paths: GraphPath[]
  laneCount: number
  height: number
  columnByHash: Map<string, number>
}

function laneX(column: number): number {
  return column * GRAPH_LANE_WIDTH + GRAPH_LANE_WIDTH / 2
}

function rowY(row: number): number {
  return row * GRAPH_ROW_HEIGHT + GRAPH_ROW_HEIGHT / 2
}

function colorForColumn(column: number, laneColor: Map<number, string>, nextColor: { value: number }): string {
  if (column === 0) return MAIN_LANE_COLOR
  if (!laneColor.has(column)) {
    laneColor.set(column, BRANCH_LANE_COLORS[nextColor.value % BRANCH_LANE_COLORS.length])
    nextColor.value += 1
  }
  return laneColor.get(column)!
}

export function isMergeCommit(commit: CommitLog): boolean {
  return commit.parents.length > 1
}

export function collectRelatedHashes(focusHash: string | null, commits: CommitLog[]): Set<string> | null {
  if (!focusHash) return null

  const known = new Set(commits.map((commit) => commit.hash))
  if (!known.has(focusHash)) return null

  const parentsByHash = new Map(commits.map((commit) => [commit.hash, commit.parents]))
  const childrenByHash = new Map<string, string[]>()
  for (const commit of commits) {
    for (const parent of commit.parents) {
      if (!known.has(parent)) continue
      const children = childrenByHash.get(parent) ?? []
      children.push(commit.hash)
      childrenByHash.set(parent, children)
    }
  }

  const related = new Set<string>([focusHash])

  const ancestorStack = [...(parentsByHash.get(focusHash) ?? [])]
  while (ancestorStack.length > 0) {
    const hash = ancestorStack.pop()!
    if (related.has(hash) || !known.has(hash)) continue
    related.add(hash)
    ancestorStack.push(...(parentsByHash.get(hash) ?? []))
  }

  const descendantStack = [...(childrenByHash.get(focusHash) ?? [])]
  while (descendantStack.length > 0) {
    const hash = descendantStack.pop()!
    if (related.has(hash) || !known.has(hash)) continue
    related.add(hash)
    descendantStack.push(...(childrenByHash.get(hash) ?? []))
  }

  return related
}

export function isPathHighlighted(path: GraphPath, focus: Set<string> | null): boolean {
  if (!focus) return true
  return focus.has(path.fromHash) && focus.has(path.toHash)
}

function pickBestRef(refs: Iterable<string>): string | null {
  const list = [...refs].sort()
  if (list.length === 0) return null
  const local = list.find((ref) => !ref.startsWith('origin/') && !ref.startsWith('remotes/'))
  if (local) return local
  return list[0].replace(/^origin\//, '').replace(/^remotes\//, '')
}

function laneLabel(column: number, refs: Set<string>): string {
  const named = pickBestRef(refs)
  if (named) return named
  return column === 0 ? '主线' : `分支 ${column}`
}

function laneId(column: number, label: string): string {
  if (label !== '主线' && !label.startsWith('分支 ')) {
    return `ref:${label}`
  }
  return `col:${column}:${label}`
}

export function buildGraphLanes(commits: CommitLog[], layout: CommitGraphLayout): GraphLane[] {
  const refsByColumn = new Map<number, Set<string>>()
  const colorByColumn = new Map<number, string>()

  for (const node of layout.nodes) {
    colorByColumn.set(node.column, node.color)
    const commit = commits.find((item) => item.hash === node.hash)
    if (!commit) continue
    const refs = refsByColumn.get(node.column) ?? new Set<string>()
    for (const ref of commit.refs) refs.add(ref)
    refsByColumn.set(node.column, refs)
  }

  const columns = [...new Set(layout.nodes.map((node) => node.column))].sort((a, b) => a - b)
  return columns.map((column) => {
    const label = laneLabel(column, refsByColumn.get(column) ?? new Set())
    return {
      id: laneId(column, label),
      column,
      label,
      color: colorByColumn.get(column) ?? MAIN_LANE_COLOR,
    }
  })
}

export function syncVisibleLaneIds(
  lanes: GraphLane[],
  visibleLaneIds: Set<string>,
  knownLaneIds: Set<string>,
): { visibleLaneIds: Set<string>; knownLaneIds: Set<string> } {
  const present = new Set(lanes.map((lane) => lane.id))
  const nextVisible = new Set([...visibleLaneIds].filter((id) => present.has(id)))
  const nextKnown = new Set(knownLaneIds)

  for (const lane of lanes) {
    if (!nextKnown.has(lane.id)) {
      nextKnown.add(lane.id)
      nextVisible.add(lane.id)
    }
  }

  if (nextVisible.size === 0) {
    for (const lane of lanes) nextVisible.add(lane.id)
  }

  return { visibleLaneIds: nextVisible, knownLaneIds: nextKnown }
}

export function visibleColumnsFromLanes(lanes: GraphLane[], visibleLaneIds: Set<string>): Set<number> {
  return new Set(
    lanes.filter((lane) => visibleLaneIds.has(lane.id)).map((lane) => lane.column),
  )
}

export function filterCommitsByColumns(
  commits: CommitLog[],
  layout: CommitGraphLayout,
  visibleColumns: Set<number>,
): CommitLog[] {
  if (visibleColumns.size === 0) return commits
  return commits.filter((commit) => visibleColumns.has(layout.columnByHash.get(commit.hash) ?? 0))
}

export function layoutCommitGraph(commits: CommitLog[]): CommitGraphLayout {
  if (commits.length === 0) {
    return {
      nodes: [],
      paths: [],
      laneCount: 1,
      height: 0,
      columnByHash: new Map(),
    }
  }

  const hashToRow = new Map<string, number>()
  commits.forEach((commit, index) => hashToRow.set(commit.hash, index))

  const columns = new Map<string, number>()
  const active: (string | null)[] = []
  const laneColor = new Map<number, string>()
  const nextColor = { value: 0 }
  let maxColumns = 1

  for (const commit of commits) {
    let column = active.indexOf(commit.hash)
    if (column === -1) {
      column = active.indexOf(null)
      if (column === -1) {
        column = active.length
        active.push(null)
      }
    }

    columns.set(commit.hash, column)
    maxColumns = Math.max(maxColumns, active.length)
    active[column] = null

    const knownParents = commit.parents.filter((parent) => hashToRow.has(parent))
    for (let i = 0; i < knownParents.length; i += 1) {
      const parent = knownParents[i]
      if (i === 0) {
        active[column] = parent
      } else {
        let newColumn = active.indexOf(null)
        if (newColumn === -1) {
          newColumn = active.length
          active.push(parent)
        } else {
          active[newColumn] = parent
        }
      }
    }

    while (active.length > 0 && active[active.length - 1] === null) {
      active.pop()
    }
  }

  const nodes: GraphNode[] = []
  const paths: GraphPath[] = []

  for (let row = 0; row < commits.length; row += 1) {
    const commit = commits[row]
    const column = columns.get(commit.hash) ?? 0
    const color = colorForColumn(column, laneColor, nextColor)
    nodes.push({ hash: commit.hash, row, column, color })

    for (const parentHash of commit.parents) {
      const parentRow = hashToRow.get(parentHash)
      if (parentRow === undefined) continue

      const parentColumn = columns.get(parentHash) ?? column
      const parentColor = colorForColumn(parentColumn, laneColor, nextColor)
      const x1 = laneX(column)
      const y1 = rowY(row)
      const x2 = laneX(parentColumn)
      const y2 = rowY(parentRow)

      if (column === parentColumn) {
        paths.push({
          d: `M ${x1} ${y1} L ${x2} ${y2}`,
          color: parentColor,
          fromHash: commit.hash,
          toHash: parentHash,
          fromColumn: column,
          toColumn: parentColumn,
          fromRow: row,
          toRow: parentRow,
        })
      } else {
        const branchY = y1 + GRAPH_ROW_HEIGHT / 2
        paths.push({
          d: `M ${x1} ${y1} L ${x1} ${branchY} L ${x2} ${branchY} L ${x2} ${y2}`,
          color: parentColor,
          fromHash: commit.hash,
          toHash: parentHash,
          fromColumn: column,
          toColumn: parentColumn,
          fromRow: row,
          toRow: parentRow,
        })
      }
    }
  }

  const columnByHash = new Map<string, number>()
  for (const node of nodes) columnByHash.set(node.hash, node.column)

  return {
    nodes,
    paths,
    laneCount: Math.max(maxColumns, 1),
    height: commits.length * GRAPH_ROW_HEIGHT,
    columnByHash,
  }
}

export function shortHash(hash: string): string {
  return hash.slice(0, 7)
}

export function formatCommitDate(date: string): string {
  const parsed = new Date(date)
  if (Number.isNaN(parsed.getTime())) return date

  const diffMs = Date.now() - parsed.getTime()
  const minutes = Math.floor(diffMs / 60_000)
  if (minutes < 1) return '刚刚'
  if (minutes < 60) return `${minutes} 分钟前`

  const hours = Math.floor(minutes / 60)
  if (hours < 24) return `${hours} 小时前`

  const days = Math.floor(hours / 24)
  if (days < 7) return `${days} 天前`

  return parsed.toLocaleString(undefined, {
    month: 'short',
    day: 'numeric',
    hour: '2-digit',
    minute: '2-digit',
  })
}

export const HISTORY_VIEW_STORAGE_KEY = 'gitwave-history-view'
