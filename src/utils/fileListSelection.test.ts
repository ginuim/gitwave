import { describe, expect, it } from 'vitest'
import { applyFileListSelectionClick } from './fileListSelection'

const paths = ['a.ts', 'b.ts', 'c.ts', 'd.ts']

describe('applyFileListSelectionClick', () => {
  it('selects only clicked file on plain click', () => {
    const result = applyFileListSelectionClick({
      sectionPaths: paths,
      current: new Set(['b.ts', 'c.ts']),
      clickedPath: 'd.ts',
      anchorPath: 'b.ts',
      shiftKey: false,
      toggleKey: false,
    })
    expect([...result.selected]).toEqual(['d.ts'])
    expect(result.anchorPath).toBe('d.ts')
  })

  it('toggles file with modifier click', () => {
    const result = applyFileListSelectionClick({
      sectionPaths: paths,
      current: new Set(['a.ts']),
      clickedPath: 'c.ts',
      anchorPath: 'a.ts',
      shiftKey: false,
      toggleKey: true,
    })
    expect([...result.selected]).toEqual(['a.ts', 'c.ts'])
  })

  it('selects range with shift click', () => {
    const result = applyFileListSelectionClick({
      sectionPaths: paths,
      current: new Set(['a.ts']),
      clickedPath: 'c.ts',
      anchorPath: 'a.ts',
      shiftKey: true,
      toggleKey: false,
    })
    expect([...result.selected]).toEqual(['a.ts', 'b.ts', 'c.ts'])
    expect(result.anchorPath).toBe('a.ts')
  })
})
