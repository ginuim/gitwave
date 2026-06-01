import type { CommitPayload } from '../types'

export function buildCommitPayload(message: string, amend: boolean): CommitPayload {
  return {
    message: message.trim(),
    amend,
  }
}

export function canSubmitCommit(message: string, loading: boolean, generating: boolean): boolean {
  return message.trim().length > 0 && !loading && !generating
}

export function commitSubmitLabel(amend: boolean, loading: boolean, success: boolean): string {
  if (loading) return '提交中...'
  if (success) return amend ? '已修正' : '已提交'
  return amend ? 'Amend' : 'Commit'
}
