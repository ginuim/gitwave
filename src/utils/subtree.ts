import type { SubtreeInfo, SubtreeRemoteConfig } from '../types'

const STORAGE_KEY = 'gitwave-subtree-remotes'

type SubtreeRemoteStore = Record<string, Record<string, SubtreeRemoteConfig>>

function loadStore(): SubtreeRemoteStore {
  try {
    const raw = localStorage.getItem(STORAGE_KEY)
    return raw ? JSON.parse(raw) : {}
  } catch {
    return {}
  }
}

function saveStore(store: SubtreeRemoteStore) {
  localStorage.setItem(STORAGE_KEY, JSON.stringify(store))
}

export function getSubtreeRemoteConfig(
  repoPath: string,
  prefix: string,
): SubtreeRemoteConfig | null {
  return loadStore()[repoPath]?.[prefix] ?? null
}

export function saveSubtreeRemoteConfig(
  repoPath: string,
  prefix: string,
  config: SubtreeRemoteConfig,
) {
  const store = loadStore()
  if (!store[repoPath]) store[repoPath] = {}
  store[repoPath][prefix] = config
  saveStore(store)
}

/** 返回路径所属的最长 subtree 前缀（嵌套前缀取更具体的） */
export function subtreePrefixForPath(path: string, subtrees: SubtreeInfo[]): string | null {
  let best: string | null = null
  for (const s of subtrees) {
    const p = s.prefix
    if (path === p || path.startsWith(`${p}/`)) {
      if (!best || p.length > best.length) best = p
    }
  }
  return best
}

export function shortCommit(hash: string | null | undefined): string {
  if (!hash) return '—'
  return hash.length > 7 ? hash.slice(0, 7) : hash
}
