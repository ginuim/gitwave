const LANE_FREQUENCIES = [196, 262, 294, 330, 392, 440, 494]
const MIN_INTERVAL_MS = 120
const PEAK_GAIN = 0.2
const TONE_DURATION_S = 0.045

export const GRAPH_HOVER_SOUND_KEY = 'gitwave-graph-hover-sound'

let audioContext: AudioContext | null = null
let lastPlayAt = 0
let lastColumn: number | null = null
let lastHash: string | null = null

function readEnabled(): boolean {
  try {
    return localStorage.getItem(GRAPH_HOVER_SOUND_KEY) !== 'off'
  } catch {
    return true
  }
}

export function isGraphHoverSoundEnabled(): boolean {
  return readEnabled()
}

export function setGraphHoverSoundEnabled(enabled: boolean): void {
  try {
    localStorage.setItem(GRAPH_HOVER_SOUND_KEY, enabled ? 'on' : 'off')
  } catch {
    // ignore storage errors
  }
}

function getAudioContext(): AudioContext | null {
  if (typeof window === 'undefined') return null
  if (!audioContext) {
    const Ctx = window.AudioContext ?? (window as typeof window & { webkitAudioContext?: typeof AudioContext }).webkitAudioContext
    if (!Ctx) return null
    audioContext = new Ctx()
  }
  return audioContext
}

export function unlockGraphHoverSound(): void {
  const ctx = getAudioContext()
  if (ctx?.state === 'suspended') {
    void ctx.resume()
  }
}

export function resetLaneHoverSound(): void {
  lastColumn = null
  lastHash = null
}

export async function playCommitHoverTone(
  column: number,
  hash: string,
  options: { multiLane: boolean },
): Promise<void> {
  try {
    if (!readEnabled()) return

    const now = performance.now()
    if (now - lastPlayAt < MIN_INTERVAL_MS) return

    if (options.multiLane) {
      if (column === lastColumn) return
    } else if (hash === lastHash) {
      return
    }

    const ctx = getAudioContext()
    if (!ctx) return

    if (ctx.state === 'suspended') {
      try {
        await ctx.resume()
      } catch {
        return
      }
    }

    if (ctx.state !== 'running') return

    const nowAfterResume = performance.now()
    if (nowAfterResume - lastPlayAt < MIN_INTERVAL_MS) return

    if (options.multiLane) {
      if (column === lastColumn) return
      lastColumn = column
    } else {
      if (hash === lastHash) return
      lastHash = hash
      lastColumn = column
    }

    lastPlayAt = nowAfterResume

    const t0 = ctx.currentTime
    const frequency = LANE_FREQUENCIES[column % LANE_FREQUENCIES.length]

    const oscillator = ctx.createOscillator()
    const gain = ctx.createGain()

    oscillator.type = 'sine'
    oscillator.frequency.setValueAtTime(frequency, t0)

    gain.gain.setValueAtTime(0.0001, t0)
    gain.gain.linearRampToValueAtTime(PEAK_GAIN, t0 + 0.006)
    gain.gain.exponentialRampToValueAtTime(0.0001, t0 + TONE_DURATION_S)

    oscillator.connect(gain)
    gain.connect(ctx.destination)

    oscillator.start(t0)
    oscillator.stop(t0 + TONE_DURATION_S + 0.01)
  } catch {
    // ignore audio failures
  }
}
