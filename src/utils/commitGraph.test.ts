import { describe, expect, it } from 'vitest'
import type { BranchTip, CommitLog } from '../types'
import { laneIdForBranchTip, layoutCommitGraph } from './commitGraph'

const commits: CommitLog[] = [
  {
    hash: 'merge',
    parents: ['main', 'feature'],
    refs: ['main'],
    author: 'tao',
    date: '2026-06-01T10:00:00Z',
    message: 'merge feature',
  },
  {
    hash: 'feature',
    parents: ['base'],
    refs: ['feature/search'],
    author: 'tao',
    date: '2026-06-01T09:00:00Z',
    message: 'feature work',
  },
  {
    hash: 'main',
    parents: ['base'],
    refs: [],
    author: 'tao',
    date: '2026-06-01T08:00:00Z',
    message: 'main work',
  },
  {
    hash: 'base',
    parents: [],
    refs: [],
    author: 'tao',
    date: '2026-06-01T07:00:00Z',
    message: 'base',
  },
]

const branchTips: BranchTip[] = [
  { name: 'feature/search', hash: 'feature' },
  { name: 'main', hash: 'merge' },
]

describe('commitGraph branch tip helpers', () => {
  it('returns the lane id that owns a branch tip commit', () => {
    const layout = layoutCommitGraph(commits)

    expect(laneIdForBranchTip('feature/search', branchTips, layout)).toBe('col:1')
    expect(laneIdForBranchTip('main', branchTips, layout)).toBe('col:0')
  })

  it('returns null for refs that are not known branch tips', () => {
    const layout = layoutCommitGraph(commits)

    expect(laneIdForBranchTip('v1.0.0', branchTips, layout)).toBeNull()
  })
})
