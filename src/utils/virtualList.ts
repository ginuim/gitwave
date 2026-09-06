export interface VirtualListWindow {
  start: number
  end: number
  top: number
  bottom: number
}

/** 按滚动位置切一段固定行高列表。start 必须夹在 [0, length]，否则滚过列表后顶栏占位会越撑越高。 */
export function virtualListWindow(
  length: number,
  scrollTop: number,
  listTop: number,
  viewportHeight: number,
  rowHeight: number,
  overscan: number,
): VirtualListWindow {
  if (length <= 0 || rowHeight <= 0) {
    return { start: 0, end: 0, top: 0, bottom: 0 }
  }

  const firstVisible = Math.floor((scrollTop - listTop) / rowHeight)
  const visibleCount = Math.max(0, Math.ceil(viewportHeight / rowHeight))
  const start = Math.min(length, Math.max(0, firstVisible - overscan))
  const end = Math.min(length, Math.max(start, firstVisible + visibleCount + overscan))
  return {
    start,
    end,
    top: start * rowHeight,
    bottom: (length - end) * rowHeight,
  }
}
