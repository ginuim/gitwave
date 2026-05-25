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
  columnByHash: Record<string, number>
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

function collectColumnLaneHashes(
  commits: CommitLog[],
  layout: CommitGraphLayout,
  column: number,
): Set<string> {
  const columnByHash = layout.columnByHash
  const known = new Set(commits.map((commit) => commit.hash))
  const queue = commits
    .filter((commit) => (columnByHash[commit.hash] ?? 0) === column)
    .map((commit) => commit.hash)
  const connected = new Set<string>()

  while (queue.length > 0) {
    const hash = queue.pop()!
    if (connected.has(hash) || !known.has(hash)) continue
    connected.add(hash)

    const commit = commits.find((item) => item.hash === hash)
    if (!commit) continue

    for (const parentHash of commit.parents) {
      if ((columnByHash[parentHash] ?? -1) === column && known.has(parentHash)) {
        queue.push(parentHash)
      }
    }

    for (const other of commits) {
      if ((columnByHash[other.hash] ?? -1) !== column) continue
      if (other.parents.includes(hash)) {
        queue.push(other.hash)
      }
    }
  }

  return connected
}

function collectColumnRefs(
  commits: CommitLog[],
  layout: CommitGraphLayout,
  column: number,
  branchTipsByHash?: Map<string, string[]>,
): Set<string> {
  const refs = new Set<string>()

  for (const hash of collectColumnLaneHashes(commits, layout, column)) {
    const commit = commits.find((item) => item.hash === hash)
    if (!commit) continue
    for (const ref of commit.refs) refs.add(ref)
    for (const name of branchTipsByHash?.get(hash) ?? []) refs.add(name)
  }

  return refs
}

function laneTipCommit(
  commits: CommitLog[],
  layout: CommitGraphLayout,
  column: number,
): CommitLog | null {
  const hashes = collectColumnLaneHashes(commits, layout, column)
  let best: CommitLog | null = null
  let bestRow = Infinity

  for (const node of layout.nodes) {
    if (!hashes.has(node.hash) || node.row >= bestRow) continue
    bestRow = node.row
    best = commits.find((commit) => commit.hash === node.hash) ?? null
  }

  return best
}

function laneLabel(
  column: number,
  refs: Set<string>,
  tipCommit: CommitLog | null,
): string {
  const named = pickBestRef(refs)
  if (named) return named
  if (column === 0) return '主线'

  const subject = tipCommit?.message.split('\n')[0]?.trim() ?? ''
  if (subject) {
    return subject.length > 32 ? `${subject.slice(0, 32)}…` : subject
  }

  return `分支 ${column}`
}

function laneId(column: number): string {
  return `col:${column}`
}

export function laneForColumn(lanes: GraphLane[], column: number): GraphLane | undefined {
  return lanes.find((lane) => lane.column === column)
}

export function buildGraphLanes(
  commits: CommitLog[],
  layout: CommitGraphLayout,
  branchTipsByHash?: Map<string, string[]>,
): GraphLane[] {
  const colorByColumn = new Map<number, string>()

  for (const node of layout.nodes) {
    colorByColumn.set(node.column, node.color)
  }

  const columns = [...new Set(layout.nodes.map((node) => node.column))].sort((a, b) => a - b)
  return columns.map((column) => {
    const refs = collectColumnRefs(commits, layout, column, branchTipsByHash)
    const tipCommit = laneTipCommit(commits, layout, column)
    return {
      id: laneId(column),
      column,
      label: laneLabel(column, refs, tipCommit),
      color: colorByColumn.get(column) ?? MAIN_LANE_COLOR,
    }
  })
}

export function syncVisibleLaneIds(
  lanes: GraphLane[],
  visibleLaneIds: string[],
  knownLaneIds: string[],
): { visibleLaneIds: string[]; knownLaneIds: string[] } {
  const present = new Set(lanes.map((lane) => lane.id))
  const nextVisible = visibleLaneIds.filter((id) => present.has(id))
  const nextKnown = [...knownLaneIds]

  for (const lane of lanes) {
    if (!nextKnown.includes(lane.id)) {
      nextKnown.push(lane.id)
      nextVisible.push(lane.id)
    }
  }

  if (nextVisible.length === 0) {
    return {
      visibleLaneIds: lanes.map((lane) => lane.id),
      knownLaneIds: nextKnown,
    }
  }

  return { visibleLaneIds: nextVisible, knownLaneIds: nextKnown }
}

export function visibleColumnsFromLanes(lanes: GraphLane[], visibleLaneIds: string[]): number[] {
  const visible = new Set(visibleLaneIds)
  return lanes.filter((lane) => visible.has(lane.id)).map((lane) => lane.column)
}

export function filterCommitsByColumns(
  commits: CommitLog[],
  layout: CommitGraphLayout,
  visibleColumns: number[],
): CommitLog[] {
  if (visibleColumns.length === 0) return commits
  const allowed = new Set(visibleColumns)
  return commits.filter((commit) => allowed.has(layout.columnByHash[commit.hash] ?? 0))
}

function appendGraphPath(
  paths: GraphPath[],
  commitHash: string,
  parentHash: string,
  row: number,
  parentRow: number,
  column: number,
  parentColumn: number,
  color: string,
): void {
  const x1 = laneX(column)
  const y1 = rowY(row)
  const x2 = laneX(parentColumn)
  const y2 = rowY(parentRow)

  if (column === parentColumn) {
    paths.push({
      d: `M ${x1} ${y1} L ${x2} ${y2}`,
      color,
      fromHash: commitHash,
      toHash: parentHash,
      fromColumn: column,
      toColumn: parentColumn,
      fromRow: row,
      toRow: parentRow,
    })
    return
  }

  const branchY = y1 + GRAPH_ROW_HEIGHT / 2
  paths.push({
    d: `M ${x1} ${y1} L ${x1} ${branchY} L ${x2} ${branchY} L ${x2} ${y2}`,
    color,
    fromHash: commitHash,
    toHash: parentHash,
    fromColumn: column,
    toColumn: parentColumn,
    fromRow: row,
    toRow: parentRow,
  })
}

function laneCountFromNodes(nodes: GraphNode[]): number {
  if (nodes.length === 0) return 1
  let maxColumn = 0
  for (const node of nodes) {
    if (node.column > maxColumn) maxColumn = node.column
  }
  return maxColumn + 1
}

export function buildFilteredGraphLayout(
  commits: CommitLog[],
  fullLayout: CommitGraphLayout,
  visibleColumns: number[],
): CommitGraphLayout {
  if (visibleColumns.length === 0) return fullLayout

  const allowed = new Set(visibleColumns)
  const filtered = commits.filter((commit) => allowed.has(fullLayout.columnByHash[commit.hash] ?? 0))

  if (filtered.length === 0) {
    return {
      nodes: [],
      paths: [],
      laneCount: 1,
      height: 0,
      columnByHash: {},
    }
  }

  const usedOriginalColumns = [...new Set(
    filtered.map((commit) => fullLayout.columnByHash[commit.hash] ?? 0),
  )].sort((a, b) => a - b)
  const columnRemap = new Map(usedOriginalColumns.map((column, index) => [column, index]))
  const hashToRow = new Map(filtered.map((commit, index) => [commit.hash, index]))
  const colorByHash = new Map(fullLayout.nodes.map((node) => [node.hash, node.color]))
  const laneColor = new Map<number, string>()
  const nextColor = { value: 0 }
  const nodes: GraphNode[] = []
  const paths: GraphPath[] = []
  const columnByHash: Record<string, number> = {}

  for (let row = 0; row < filtered.length; row += 1) {
    const commit = filtered[row]
    const originalColumn = fullLayout.columnByHash[commit.hash] ?? 0
    const column = columnRemap.get(originalColumn) ?? 0
    const color = colorByHash.get(commit.hash) ?? colorForColumn(column, laneColor, nextColor)
    nodes.push({ hash: commit.hash, row, column, color })
    columnByHash[commit.hash] = column

    for (const parentHash of commit.parents) {
      const parentRow = hashToRow.get(parentHash)
      if (parentRow === undefined) continue

      const parentOriginalColumn = fullLayout.columnByHash[parentHash] ?? originalColumn
      const parentColumn = columnRemap.get(parentOriginalColumn) ?? column
      const parentColor = colorByHash.get(parentHash)
        ?? colorByHash.get(commit.hash)
        ?? colorForColumn(parentColumn, laneColor, nextColor)
      appendGraphPath(paths, commit.hash, parentHash, row, parentRow, column, parentColumn, parentColor)
    }
  }

  return {
    nodes,
    paths,
    laneCount: laneCountFromNodes(nodes),
    height: filtered.length * GRAPH_ROW_HEIGHT,
    columnByHash,
  }
}

export function layoutCommitGraph(commits: CommitLog[]): CommitGraphLayout {
  if (commits.length === 0) {
    return {
      nodes: [],
      paths: [],
      laneCount: 1,
      height: 0,
      columnByHash: {},
    }
  }

  const hashToRow = new Map<string, number>()
  commits.forEach((commit, index) => hashToRow.set(commit.hash, index))

  const columns = new Map<string, number>()
  const active: (string | null)[] = []
  const laneColor = new Map<number, string>()
  const nextColor = { value: 0 }

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
    active[column] = null

    const knownParents = commit.parents.filter((parent) => hashToRow.has(parent))
    for (let i = 0; i < knownParents.length; i += 1) {
      const parent = knownParents[i]
      if (active.indexOf(parent) !== -1) continue
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

  const usedColumns = [...new Set(columns.values())].sort((a, b) => a - b)
  const compactColumn = new Map(usedColumns.map((col, index) => [col, index]))
  const remapColumn = (col: number) => compactColumn.get(col) ?? 0

  const nodes: GraphNode[] = []
  const paths: GraphPath[] = []

  for (let row = 0; row < commits.length; row += 1) {
    const commit = commits[row]
    const column = remapColumn(columns.get(commit.hash) ?? 0)
    const color = colorForColumn(column, laneColor, nextColor)
    nodes.push({ hash: commit.hash, row, column, color })

    for (const parentHash of commit.parents) {
      const parentRow = hashToRow.get(parentHash)
      if (parentRow === undefined) continue

      const parentColumn = remapColumn(columns.get(parentHash) ?? column)
      const parentColor = colorForColumn(parentColumn, laneColor, nextColor)
      appendGraphPath(paths, commit.hash, parentHash, row, parentRow, column, parentColumn, parentColor)
    }
  }

  const columnByHash: Record<string, number> = {}
  for (const node of nodes) columnByHash[node.hash] = node.column

  return {
    nodes,
    paths,
    laneCount: laneCountFromNodes(nodes),
    height: commits.length * GRAPH_ROW_HEIGHT,
    columnByHash,
  }
}

export function distinctColumnCount(columnByHash: Record<string, number>): number {
  const columns = Object.values(columnByHash)
  if (columns.length === 0) return 0
  return new Set(columns).size
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
