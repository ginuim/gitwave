import { readFileSync } from 'node:fs'
import { fileURLToPath } from 'node:url'
import { describe, expect, it } from 'vitest'

const diffPanelSource = readFileSync(
  fileURLToPath(new URL('../src/components/DiffPanel.vue', import.meta.url)),
  'utf8',
)

describe('DiffPanel sticky action bars', () => {
  it('keeps sticky header heights aligned with the change block offset', () => {
    expect(diffPanelSource).not.toContain('top-22')
    expect(diffPanelSource).toContain('sticky top-0 z-10 flex h-12')
    expect(diffPanelSource).toContain('sticky top-12 z-[5] flex h-10')
    expect(diffPanelSource).toContain('sticky top-[5.5rem] z-[4]')
    expect(diffPanelSource).not.toContain('z-[6]')
  })
})
