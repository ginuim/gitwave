import { describe, expect, it } from 'vitest'
import {
  applyOptimisticRevertToDiffText,
  buildPatchForSegment,
  buildPatchForSelection,
  getHunkBodySegments,
  lineKeysInChangeSegment,
  parseDiffSections,
} from './diffPatch'

const SAMPLE_DIFF = `diff --git a/src/demo.ts b/src/demo.ts
index 1111111..2222222 100644
--- a/src/demo.ts
+++ b/src/demo.ts
@@ -1,7 +1,8 @@
 const a = 1
-const b = 2
+const b = 20
+const c = 3
 const d = 4
-const e = 5
+const e = 50
 const f = 6
`

describe('diffPatch', () => {
  it('parses stable line keys and line numbers from a unified diff', () => {
    const [section] = parseDiffSections(SAMPLE_DIFF, null)
    const [hunk] = section.hunks

    expect(section.fileName).toBe('src/demo.ts')
    expect(hunk.lines.map((line) => line.key)).toEqual([
      '0:0:0',
      '0:0:1',
      '0:0:2',
      '0:0:3',
      '0:0:4',
      '0:0:5',
      '0:0:6',
      '0:0:7',
      '0:0:8',
    ])
    expect(hunk.lines.map((line) => [line.oldLineNumber, line.newLineNumber])).toEqual([
      [null, null],
      [1, 1],
      [2, null],
      [null, 2],
      [null, 3],
      [3, 4],
      [4, null],
      [null, 5],
      [5, 6],
    ])
  })

  it('treats continuous +/- runs as Sourcetree-style change blocks', () => {
    const [section] = parseDiffSections(SAMPLE_DIFF, null)
    const [hunk] = section.hunks

    expect(getHunkBodySegments(hunk)).toEqual([
      { kind: 'context', startLineIndex: 1, endLineIndex: 1 },
      { kind: 'changes', startLineIndex: 2, endLineIndex: 4 },
      { kind: 'context', startLineIndex: 5, endLineIndex: 5 },
      { kind: 'changes', startLineIndex: 6, endLineIndex: 7 },
      { kind: 'context', startLineIndex: 8, endLineIndex: 8 },
    ])
  })

  it('builds a block patch from only the selected change segment', () => {
    const [section] = parseDiffSections(SAMPLE_DIFF, null)
    const [hunk] = section.hunks
    const segment = getHunkBodySegments(hunk).find((item) => item.kind === 'changes')!

    const patch = buildPatchForSegment(section, hunk, segment, 'stage')

    expect(patch).toContain('-const b = 2')
    expect(patch).toContain('+const b = 20')
    expect(patch).toContain('+const c = 3')
    expect(patch).toContain(' const e = 5')
    expect(patch).not.toContain('-const e = 5')
    expect(patch).not.toContain('+const e = 50')
  })

  it('builds a line patch from explicit line keys across change blocks', () => {
    const [section] = parseDiffSections(SAMPLE_DIFF, null)
    const [hunk] = section.hunks
    const segments = getHunkBodySegments(hunk).filter((item) => item.kind === 'changes')
    const selected = new Set<string>([
      ...lineKeysInChangeSegment(hunk, segments[0]).values(),
      hunk.lines[7].key,
    ])

    const patch = buildPatchForSelection([section], selected, 'stage')

    expect(patch).toContain('-const b = 2')
    expect(patch).toContain('+const b = 20')
    expect(patch).toContain('+const c = 3')
    expect(patch).toContain('+const e = 50')
    expect(patch).toContain(' const e = 5')
  })

  it('optimistically removes reverted +/- lines from diff text', () => {
    const [section] = parseDiffSections(SAMPLE_DIFF, null)
    const [hunk] = section.hunks
    const segment = getHunkBodySegments(hunk).find((item) => item.kind === 'changes')!
    const revertedKeys = lineKeysInChangeSegment(hunk, segment)

    const next = applyOptimisticRevertToDiffText(SAMPLE_DIFF, null, revertedKeys)

    expect(next).not.toContain('-const b = 2')
    expect(next).not.toContain('+const b = 20')
    expect(next).not.toContain('+const c = 3')
    expect(next).toContain('-const e = 5')
    expect(next).toContain('+const e = 50')
  })

  it('keeps reverted removed lines as context so later line numbers do not shift', () => {
    const [section] = parseDiffSections(SAMPLE_DIFF, null)
    const [hunk] = section.hunks
    const removedLine = hunk.lines.find((line) => line.content === '-const b = 2')!

    const next = applyOptimisticRevertToDiffText(SAMPLE_DIFF, null, new Set([removedLine.key]))
    const [nextSection] = parseDiffSections(next, null)
    const [nextHunk] = nextSection.hunks

    expect(next).toContain(' const b = 2')
    expect(next).not.toContain('-const b = 2')
    expect(nextHunk.lines.find((line) => line.content === ' const d = 4')).toMatchObject({
      oldLineNumber: 3,
      newLineNumber: 5,
    })
    expect(next).toContain('-const e = 5')
    expect(next).toContain('+const e = 50')
  })

  const TWO_FILE_DIFF = `diff --git a/a.ts b/a.ts
index 1111111..2222222 100644
--- a/a.ts
+++ b/a.ts
@@ -1,1 +1,1 @@
-old-a
+new-a
diff --git a/b.ts b/b.ts
index 3333333..4444444 100644
--- a/b.ts
+++ b/b.ts
@@ -1,1 +1,1 @@
-old-b
+new-b
`

  it('keeps every file when the concatenated diff starts with diff --git', () => {
    const sections = parseDiffSections(TWO_FILE_DIFF, null)
    expect(sections.map((section) => section.fileName)).toEqual(['a.ts', 'b.ts'])
  })

  it('does not drop the first file when optimistically reverting a later section', () => {
    const sections = parseDiffSections(TWO_FILE_DIFF, null)
    const revertedKeys = lineKeysInChangeSegment(sections[1].hunks[0], {
      kind: 'changes',
      startLineIndex: 1,
      endLineIndex: 2,
    })
    const next = applyOptimisticRevertToDiffText(TWO_FILE_DIFF, null, revertedKeys)
    const nextSections = parseDiffSections(next, null)
    expect(nextSections.map((section) => section.fileName)).toEqual(['a.ts'])
    expect(next).toContain('+new-a')
    expect(next).not.toContain('+new-b')
  })
})
