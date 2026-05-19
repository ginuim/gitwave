import type { FileStatus } from '../types'

/** porcelain `??`：未跟踪的新文件 */
export function isUntrackedFile(file: FileStatus): boolean {
  return !file.isStaged && file.status.trim() === '??'
}

export function isUntrackedPath(path: string, statuses: FileStatus[]): boolean {
  const entry = statuses.find((s) => s.path === path && !s.isStaged)
  return entry != null && isUntrackedFile(entry)
}

/** `git diff --no-index` 等新文件 diff */
export function diffShowsNewFile(diffText: string): boolean {
  return diffText.includes('--- /dev/null') || diffText.includes('new file mode')
}
