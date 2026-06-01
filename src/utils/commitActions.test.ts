import { describe, expect, it } from 'vitest'
import { buildCommitPayload, commitSubmitLabel, canSubmitCommit } from './commitActions'

describe('commit action helpers', () => {
  it('builds a normal commit payload from a trimmed message', () => {
    expect(buildCommitPayload('  add shortcuts  ', false)).toEqual({
      message: 'add shortcuts',
      amend: false,
    })
  })

  it('builds an amend payload without inventing another action type', () => {
    expect(buildCommitPayload('fix last commit', true)).toEqual({
      message: 'fix last commit',
      amend: true,
    })
  })

  it('uses amend-specific labels and keeps empty messages disabled', () => {
    expect(commitSubmitLabel(false, false, false)).toBe('Commit')
    expect(commitSubmitLabel(true, false, false)).toBe('Amend')
    expect(commitSubmitLabel(true, true, false)).toBe('提交中...')
    expect(canSubmitCommit('   ', false, false)).toBe(false)
  })
})
