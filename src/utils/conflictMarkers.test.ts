import { describe, expect, it } from 'vitest'
import {
  parseConflictBlocks,
  resolveConflictBlock,
} from './conflictMarkers'

const CONFLICTED = `line 1
<<<<<<< HEAD
ours a
ours b
=======
theirs a
>>>>>>> feature
line 2
<<<<<<< HEAD
ours c
=======
theirs c
theirs d
>>>>>>> feature
line 3
`

describe('conflictMarkers', () => {
  it('parses conflict marker blocks with line positions and sides', () => {
    const blocks = parseConflictBlocks(CONFLICTED)

    expect(blocks).toEqual([
      {
        index: 0,
        startLine: 1,
        separatorLine: 4,
        endLine: 6,
        ours: ['ours a', 'ours b'],
        theirs: ['theirs a'],
      },
      {
        index: 1,
        startLine: 8,
        separatorLine: 10,
        endLine: 13,
        ours: ['ours c'],
        theirs: ['theirs c', 'theirs d'],
      },
    ])
  })

  it('resolves only the selected block with ours', () => {
    const next = resolveConflictBlock(CONFLICTED, 1, 'ours')

    expect(next).toContain('<<<<<<< HEAD')
    expect(next).toContain('theirs a')
    expect(next).toContain('line 2\nours c\nline 3')
    expect(next).not.toContain('theirs c')
  })

  it('resolves a block with both sides in order', () => {
    const next = resolveConflictBlock(CONFLICTED, 0, 'both')

    expect(next).toContain('line 1\nours a\nours b\ntheirs a\nline 2')
    expect(next).toContain('<<<<<<< HEAD\nours c')
  })
})
