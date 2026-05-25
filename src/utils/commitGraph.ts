import type { CommitLog } from '../types'

export const GRAPH_ROW_HEIGHT = 56
export const GRAPH_LANE_WIDTH = 14
export const GRAPH_NODE_RADIUS = 4

const LANE_COLORS = [
  '#3b82f6',
  '#f59e0b',
  '#22c55e',
  '#ef4444',
  '#a855f7',
  '#ec4899',
  '#06b6d4',
  '#84cc16',
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
}

export interface CommitGraphLayout {
  nodes: GraphNode[]
  paths: GraphPath[]
  laneCount: number
  height: number
}

function laneX(column: number): number {
  return column * GRAPH_LANE_WIDTH + GRAPH_LANE_WIDTH / 2
}

function rowY(row: number): number {
  return row * GRAPH_ROW_HEIGHT + GRAPH_ROW_HEIGHT / 2
}

function colorForColumn(column: number, laneColor: Map<number, string>, nextColor: { value: number }): string {
  if (!laneColor.has(column)) {
    laneColor.set(column, LANE_COLORS[nextColor.value % LANE_COLORS.length])
    nextColor.value += 1
  }
  return laneColor.get(column)!
}

export function layoutCommitGraph(commits: CommitLog[]): CommitGraphLayout {
  if (commits.length === 0) {
    return { nodes: [], paths: [], laneCount: 1, height: 0 }
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
        })
      } else {
        const branchY = y1 + GRAPH_ROW_HEIGHT / 2
        paths.push({
          d: `M ${x1} ${y1} L ${x1} ${branchY} L ${x2} ${branchY} L ${x2} ${y2}`,
          color: parentColor,
        })
      }
    }
  }

  return {
    nodes,
    paths,
    laneCount: Math.max(maxColumns, 1),
    height: commits.length * GRAPH_ROW_HEIGHT,
  }
}

export function shortHash(hash: string): string {
  return hash.slice(0, 7)
}

export function formatCommitDate(date: string): string {
  const parsed = new Date(date)
  if (Number.isNaN(parsed.getTime())) return date
  return parsed.toLocaleString(undefined, {
    month: 'short',
    day: 'numeric',
    hour: '2-digit',
    minute: '2-digit',
  })
}
