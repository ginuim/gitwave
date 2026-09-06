import { describe, expect, it } from 'vitest'
import { virtualListWindow } from './virtualList'

const ROW = 48
const OVERSCAN = 8
const VIEWPORT = 480

describe('virtualListWindow', () => {
  it('keeps total height equal to length * rowHeight', () => {
    const win = virtualListWindow(40, 200, 0, VIEWPORT, ROW, OVERSCAN)
    expect(win.top + (win.end - win.start) * ROW + win.bottom).toBe(40 * ROW)
  })

  it('starts at 0 when the list has not reached the viewport', () => {
    const win = virtualListWindow(20, 0, 2000, VIEWPORT, ROW, OVERSCAN)
    expect(win.start).toBe(0)
    expect(win.end).toBe(0)
    expect(win.top).toBe(0)
    expect(win.bottom).toBe(20 * ROW)
  })

  it('does not grow the spacer after scrolling past the list', () => {
    const length = 10
    const win = virtualListWindow(length, 4000, 0, VIEWPORT, ROW, OVERSCAN)
    expect(win.start).toBe(length)
    expect(win.end).toBe(length)
    expect(win.top).toBe(length * ROW)
    expect(win.bottom).toBe(0)
  })

  it('clamps a negative firstVisible index to the top of the list', () => {
    const win = virtualListWindow(30, 10, 80, VIEWPORT, ROW, OVERSCAN)
    expect(win.start).toBe(0)
    expect(win.end).toBeGreaterThan(0)
    expect(win.end).toBeLessThanOrEqual(30)
  })

  it('returns an empty window for an empty list', () => {
    expect(virtualListWindow(0, 120, 0, VIEWPORT, ROW, OVERSCAN)).toEqual({
      start: 0,
      end: 0,
      top: 0,
      bottom: 0,
    })
  })
})
