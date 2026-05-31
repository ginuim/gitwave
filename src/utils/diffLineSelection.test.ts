import { describe, expect, it } from 'vitest'
import {
  applyLineSelectionClick,
  lineRangeSelection,
  type SelectableLineRef,
} from './diffLineSelection'

const lines: SelectableLineRef[] = [
  { id: '0:0:2' },
  { id: '0:0:3' },
  { id: '0:0:4' },
  { id: '0:0:7' },
]

describe('diffLineSelection', () => {
  it('selects one line and makes it the anchor', () => {
    const result = applyLineSelectionClick({
      lines,
      current: new Set(),
      clickedId: '0:0:3',
      anchorId: null,
      shiftKey: false,
      toggleKey: false,
    })

    expect([...result.selected]).toEqual(['0:0:3'])
    expect(result.anchorId).toBe('0:0:3')
  })

  it('selects a stable range from anchor to hovered line', () => {
    expect([...lineRangeSelection(lines, '0:0:2', '0:0:7')]).toEqual([
      '0:0:2',
      '0:0:3',
      '0:0:4',
      '0:0:7',
    ])
  })

  it('toggles a line without losing other selected lines', () => {
    const result = applyLineSelectionClick({
      lines,
      current: new Set(['0:0:2', '0:0:3']),
      clickedId: '0:0:3',
      anchorId: '0:0:2',
      shiftKey: false,
      toggleKey: true,
    })

    expect([...result.selected]).toEqual(['0:0:2'])
    expect(result.anchorId).toBe('0:0:3')
  })

  it('deselects a selected line on plain click', () => {
    const result = applyLineSelectionClick({
      lines,
      current: new Set(['0:0:3']),
      clickedId: '0:0:3',
      anchorId: '0:0:3',
      shiftKey: false,
      toggleKey: false,
    })

    expect([...result.selected]).toEqual([])
    expect(result.anchorId).toBeNull()
  })

  it('deselects one line from a multi selection on plain click', () => {
    const result = applyLineSelectionClick({
      lines,
      current: new Set(['0:0:2', '0:0:3', '0:0:4']),
      clickedId: '0:0:3',
      anchorId: '0:0:2',
      shiftKey: false,
      toggleKey: false,
    })

    expect([...result.selected]).toEqual(['0:0:2', '0:0:4'])
    expect(result.anchorId).toBe('0:0:3')
  })
})
