<script setup lang="ts">
import { ref, computed, onMounted, onUnmounted, watch } from 'vue'
import { invoke } from '@tauri-apps/api/core'
import { fetch } from '@tauri-apps/plugin-http'
import { join } from '@tauri-apps/api/path'
import { revealItemInDir } from '@tauri-apps/plugin-opener'
import { FilePlus, FileMinus, FolderOpen, GitCommitVertical, Loader2, Sparkles, AlertCircle, Check, Settings, Undo2, Trash2, RotateCcw } from 'lucide-vue-next'
import type { FileStatus, AppSettings, ProviderConfig, ModelConfig, AiStagedDiffContext, SubtreeInfo, CommitPayload } from '../types'
import { isUntrackedFile, isUntrackedPath } from '../utils/gitStatus'
import { subtreePrefixForPath } from '../utils/subtree'
import { buildCommitPayload, canSubmitCommit, commitSubmitLabel } from '../utils/commitActions'

const props = defineProps<{
  statuses: FileStatus[]
  selectedFile: string | null
  selectedFiles: string[]
  commitLoading: boolean
  /** 父组件在提交成功后递增，用于显示「已提交」短提示 */
  commitSuccessTick: number
  statusLoading: boolean
  repoPath: string | null
  settingsRevision: number
  subtrees: SubtreeInfo[]
  conflictedFiles?: string[]
}>()

const emit = defineEmits<{
  stageFile: [paths: string[]]
  unstageFile: [paths: string[]]
  revertFile: [paths: string[], isStaged: boolean]
  deleteFile: [paths: string[], isStaged: boolean]
  selectFile: [path: string, isStaged: boolean, modifiers: { shiftKey: boolean; toggleKey: boolean; sectionPaths: string[] }]
  commit: [payload: CommitPayload]
  softResetLastCommit: []
  revealError: [message: string]
  openSettings: []
}>()

// === Commit area resize ===
const COMMIT_AREA_HEIGHT_KEY = 'gitwave-commit-area-height'
const DEFAULT_COMMIT_AREA_HEIGHT = 220
const MIN_COMMIT_AREA_HEIGHT = 140
const MAX_COMMIT_AREA_HEIGHT = 560

const panelRef = ref<HTMLElement | null>(null)
const workspaceScrollRef = ref<HTMLElement | null>(null)
const unstagedListRef = ref<HTMLElement | null>(null)
const stagedListRef = ref<HTMLElement | null>(null)
const commitAreaHeight = ref(DEFAULT_COMMIT_AREA_HEIGHT)
const workspaceScrollTop = ref(0)
const FILE_ROW_HEIGHT = 48
const FILE_ROW_OVERSCAN = 8

function clampCommitAreaHeight(height: number): number {
  const panelMax = panelRef.value
    ? Math.floor(panelRef.value.clientHeight * 0.65)
    : MAX_COMMIT_AREA_HEIGHT
  const max = Math.min(MAX_COMMIT_AREA_HEIGHT, panelMax)
  return Math.min(max, Math.max(MIN_COMMIT_AREA_HEIGHT, height))
}

let resizeStartY = 0
let resizeStartHeight = 0
let resizeMoveHandler: ((e: MouseEvent) => void) | null = null
let resizeUpHandler: (() => void) | null = null

function stopCommitAreaResize() {
  if (resizeMoveHandler) {
    document.removeEventListener('mousemove', resizeMoveHandler)
    resizeMoveHandler = null
  }
  if (resizeUpHandler) {
    document.removeEventListener('mouseup', resizeUpHandler)
    resizeUpHandler = null
  }
  document.body.style.cursor = ''
  document.body.style.userSelect = ''
}

function onCommitAreaResizeStart(e: MouseEvent) {
  e.preventDefault()
  resizeStartY = e.clientY
  resizeStartHeight = commitAreaHeight.value
  stopCommitAreaResize()

  resizeMoveHandler = (ev: MouseEvent) => {
    const delta = resizeStartY - ev.clientY
    commitAreaHeight.value = clampCommitAreaHeight(resizeStartHeight + delta)
  }
  resizeUpHandler = () => {
    stopCommitAreaResize()
    localStorage.setItem(COMMIT_AREA_HEIGHT_KEY, String(commitAreaHeight.value))
  }

  document.body.style.cursor = 'ns-resize'
  document.body.style.userSelect = 'none'
  document.addEventListener('mousemove', resizeMoveHandler)
  document.addEventListener('mouseup', resizeUpHandler)
}

onUnmounted(stopCommitAreaResize)

// === Manual commit state ===
const commitMessage = ref('')
const amendLastCommit = ref(false)
const commitSuccess = ref(false)
let commitSuccessTimer: ReturnType<typeof setTimeout> | null = null

watch(() => props.commitLoading, (loading) => {
  if (loading && commitSuccess.value) {
    commitSuccess.value = false
    if (commitSuccessTimer) {
      clearTimeout(commitSuccessTimer)
      commitSuccessTimer = null
    }
  }
})

watch(() => props.commitSuccessTick, (tick, prevTick) => {
  if (tick <= 0) return
  if (prevTick !== undefined && tick <= prevTick) return
  commitSuccess.value = true
  if (commitSuccessTimer) clearTimeout(commitSuccessTimer)
  commitSuccessTimer = setTimeout(() => {
    commitSuccess.value = false
    commitSuccessTimer = null
  }, 2000)
})

function handleCommit(amend = amendLastCommit.value) {
  if (!canSubmitCommit(commitMessage.value, props.commitLoading, generating.value)) return
  emit('commit', buildCommitPayload(commitMessage.value, amend))
  commitMessage.value = ''
}

function handleCommitKeydown(e: KeyboardEvent) {
  if (e.key !== 'Enter') return
  if (!e.metaKey && !e.ctrlKey) return
  e.preventDefault()
  handleCommit(e.shiftKey ? true : amendLastCommit.value)
}

// === AI commit state ===
const aiSettings = ref<AppSettings | null>(null)
const generating = ref(false)
const aiLoading = ref(true)
const aiError = ref<string | null>(null)
const selectedModelId = ref<string>('')

const allModels = computed<(ModelConfig & { provider: ProviderConfig })[]>(() => {
  if (!aiSettings.value) return []
  const result: (ModelConfig & { provider: ProviderConfig })[] = []
  for (const prov of aiSettings.value.providers) {
    for (const model of aiSettings.value.models.filter(m => m.providerId === prov.id)) {
      result.push({ ...model, provider: prov })
    }
  }
  return result
})

const selectedModel = computed(() => allModels.value.find(m => m.id === selectedModelId.value) || null)

const defaultModel = computed(() => allModels.value.find(m => m.isDefault) || allModels.value[0] || null)

const hasStagedFiles = computed(() => props.statuses.some(f => f.isStaged))

const commitPrompt = computed(() => aiSettings.value?.prompts?.commitPrompt || '')

onMounted(async () => {
  const stored = localStorage.getItem(COMMIT_AREA_HEIGHT_KEY)
  if (stored) {
    const parsed = Number(stored)
    if (!Number.isNaN(parsed)) commitAreaHeight.value = clampCommitAreaHeight(parsed)
  }
  if (props.repoPath) await loadAiData()
})

watch(() => props.settingsRevision, () => {
  if (!generating.value) reloadAiSettings()
})

// Reload when repo path becomes available (e.g. after app init)
watch(() => props.repoPath, (path) => {
  if (path && aiSettings.value === null) {
    loadAiData()
  }
})

async function reloadAiSettings() {
  try {
    const s = await invoke<AppSettings>('load_settings')
    aiSettings.value = s
    if (selectedModelId.value && !allModels.value.find(m => m.id === selectedModelId.value)) {
      const d = defaultModel.value
      if (d) selectedModelId.value = d.id
    } else if (allModels.value.length > 0 && !selectedModelId.value) {
      const d = defaultModel.value
      if (d) selectedModelId.value = d.id
    }
  } catch (_) { /* silent */ }
}

async function loadAiData() {
  aiLoading.value = true
  aiError.value = null
  try {
    const s = await invoke<AppSettings>('load_settings')
    aiSettings.value = s
    if (!selectedModelId.value || !allModels.value.find(m => m.id === selectedModelId.value)) {
      const d = defaultModel.value
      if (d) selectedModelId.value = d.id
      else if (allModels.value.length > 0) selectedModelId.value = allModels.value[0].id
    }
  } catch (e: any) {
    aiError.value = String(e)
  } finally {
    aiLoading.value = false
  }
}

async function generateCommitMessage() {
  if (!selectedModel.value || generating.value) return
  generating.value = true
  aiError.value = null
  commitMessage.value = ''

  const t0 = performance.now()
  const log = (msg: string) => console.log(`[AI] +${(performance.now() - t0).toFixed(0)}ms ${msg}`)

  try {
    const model = selectedModel.value
    log(`start — model=${model.name} provider=${model.provider.type} baseUrl=${model.provider.baseUrl}`)

    const ctx = await invoke<AiStagedDiffContext>('get_staged_diff_for_ai')
    const prompt = buildPrompt(ctx)
    log(`prompt built — ${prompt.length} chars, branch=${ctx.currentBranch}, summary=${ctx.summary.length} chars, diffs=${ctx.fileDiffs.length}, omitted=${ctx.omittedFiles.length}`)

    if (model.provider.type === 'openai') {
      await streamOpenAI(model.provider, model.name, prompt, log)
    } else {
      await streamAnthropic(model.provider, model.name, prompt, log)
    }
    commitMessage.value = commitMessage.value.trimStart()
    log(`done — total=${commitMessage.value.length} chars`)
  } catch (e: any) {
    console.error('[AI] error', e)
    aiError.value = String(e)
  } finally {
    generating.value = false
  }
}

function buildPrompt(ctx: AiStagedDiffContext): string {
  const prompt = commitPrompt.value || ''
  return `You are a git commit message generator.

${prompt ? `## Commit Convention\n${prompt}\n\n` : ''}${ctx.promptBody}

Please generate a commit message for the above staged changes following the conventions described above.
Return ONLY the commit message, nothing else.`
}

// --- SSE streaming helpers ---

function parseSSELine(line: string): { event?: string; data?: string } | null {
  if (line.startsWith('event: ')) return { event: line.slice(7).trim() }
  if (line.startsWith('data: ')) return { data: line.slice(6) }
  return null
}

async function streamOpenAI(provider: ProviderConfig, model: string, prompt: string, log: (msg: string) => void): Promise<void> {
  const baseUrl = provider.baseUrl.replace(/\/+$/, '')
  const url = baseUrl.includes('/chat/completions') ? baseUrl : `${baseUrl}/chat/completions`
  log(`fetch → ${url}`)

  const resp = await fetch(url, {
    method: 'POST',
    headers: {
      'Content-Type': 'application/json',
      'Authorization': `Bearer ${provider.apiKey}`,
    },
    body: JSON.stringify({
      model,
      messages: [{ role: 'user', content: prompt }],
      max_tokens: 512,
      temperature: 0.3,
      stream: true,
    }),
  })
  log(`response received — status=${resp.status} hasBody=${!!resp.body}`)

  if (!resp.ok) {
    const text = await resp.text()
    throw new Error(`OpenAI API error (${resp.status}): ${text}`)
  }

  const reader = resp.body?.getReader()
  if (!reader) {
    log('no reader — falling back to full response')
    const rawText = await resp.text()
    let data: unknown
    try {
      data = JSON.parse(rawText)
    } catch {
      throw new Error(`OpenAI returned non-JSON response: ${rawText.slice(0, 500)}`)
    }
    const d = data as Record<string, unknown>
    const fromChoices = (d?.choices as { message?: { content?: string } }[] | undefined)?.[0]?.message?.content
    commitMessage.value = (fromChoices || (d?.response as string) || (d?.text as string) || '').trim()
    return
  }

  log('reader acquired — starting stream read')
  const decoder = new TextDecoder()
  let buffer = ''
  let contentStarted = false
  let chunkCount = 0

  while (true) {
    const { done, value } = await reader.read()
    if (done) {
      log(`stream done — ${chunkCount} chunks received`)
      break
    }
    chunkCount++
    buffer += decoder.decode(value, { stream: true })

    const lines = buffer.split('\n')
    buffer = lines.pop() || ''

    for (const line of lines) {
      const trimmed = line.trim()
      if (!trimmed || trimmed === 'data: [DONE]') continue
      if (!trimmed.startsWith('data: ')) continue

      try {
        const data = JSON.parse(trimmed.slice(6))
        const token = data?.choices?.[0]?.delta?.content
        if (!token) continue
        if (!contentStarted) {
          const trimmedToken = token.replace(/^\s+/, '')
          if (!trimmedToken) continue
          log(`first token arrived — chunk #${chunkCount}`)
          commitMessage.value += trimmedToken
          contentStarted = true
        } else {
          commitMessage.value += token
        }
      } catch { /* skip malformed JSON lines */ }
    }
  }
}

async function streamAnthropic(provider: ProviderConfig, model: string, prompt: string, log: (msg: string) => void): Promise<void> {
  const baseUrl = provider.baseUrl.replace(/\/+$/, '')
  const url = baseUrl.includes('/v1/messages') ? baseUrl : `${baseUrl}/v1/messages`
  log(`fetch → ${url}`)

  const resp = await fetch(url, {
    method: 'POST',
    headers: {
      'Content-Type': 'application/json',
      'x-api-key': provider.apiKey,
      'anthropic-version': '2023-06-01',
    },
    body: JSON.stringify({
      model,
      max_tokens: 4096,
      messages: [{ role: 'user', content: prompt }],
      stream: true,
    }),
  })
  log(`response received — status=${resp.status} hasBody=${!!resp.body}`)

  if (!resp.ok) {
    const text = await resp.text()
    throw new Error(`Anthropic API error (${resp.status}): ${text}`)
  }

  const reader = resp.body?.getReader()
  if (!reader) {
    log('no reader — falling back to full response')
    const rawText = await resp.text()
    let data: unknown
    try {
      data = JSON.parse(rawText)
    } catch {
      throw new Error(`Anthropic returned non-JSON response: ${rawText.slice(0, 500)}`)
    }
    const d = data as Record<string, unknown>
    if (Array.isArray(d?.content)) {
      for (const block of d.content as { type?: string; text?: string }[]) {
        if (block?.type === 'text' && block?.text) {
          commitMessage.value = block.text
          return
        }
      }
    }
    const c = d?.content
    commitMessage.value = (
      (typeof c === 'object' && c && 'text' in c ? (c as { text: string }).text : '') ||
      (typeof c === 'string' ? c : '') ||
      String(d?.completion ?? d?.text ?? '')
    ).trim()
    return
  }

  log('reader acquired — starting stream read')
  const decoder = new TextDecoder()
  let buffer = ''
  let currentEvent = ''
  let contentStarted = false
  let chunkCount = 0

  while (true) {
    const { done, value } = await reader.read()
    if (done) {
      log(`stream done — ${chunkCount} chunks received`)
      break
    }
    chunkCount++
    const raw = decoder.decode(value, { stream: true })
    if (chunkCount <= 3) {
      log(`chunk #${chunkCount} arrived (${raw.length} bytes)`)
    }
    buffer += raw

    const lines = buffer.split('\n')
    buffer = lines.pop() || ''

    for (const line of lines) {
      const trimmed = line.trim()
      if (!trimmed) continue

      const parsed = parseSSELine(trimmed)
      if (!parsed) continue
      if (parsed.event) { currentEvent = parsed.event; continue }
      if (!parsed.data || parsed.data === '[DONE]') continue

      try {
        const data = JSON.parse(parsed.data)
        let token = ''
        if (currentEvent === 'content_block_delta' && data?.delta?.text) {
          token = data.delta.text
        } else if (currentEvent === 'content_block_start' && data?.content_block?.text) {
          token = data.content_block.text
        }
        // Fallback: OpenAI-style delta inside Anthropic-compat endpoint
        if (!token && data?.choices?.[0]?.delta?.content) {
          token = data.choices[0].delta.content
          if (!contentStarted) log(`detected OpenAI-style delta in Anthropic endpoint`)
        }
        if (!token) continue
        if (!contentStarted) {
          const trimmedToken = token.replace(/^\s+/, '')
          if (!trimmedToken) continue
          log(`first token arrived — chunk #${chunkCount} event=${currentEvent}`)
          commitMessage.value += trimmedToken
          contentStarted = true
        } else {
          commitMessage.value += token
        }
      } catch { /* skip malformed JSON */ }
    }
  }
}

// === File list helpers ===
const unstagedFiles = computed(() => props.statuses.filter((f) => !f.isStaged))
const stagedFiles = computed(() => props.statuses.filter((f) => f.isStaged))
const conflictedFileSet = computed(() => new Set(props.conflictedFiles ?? []))
const selectedFileSet = computed(() => new Set(props.selectedFiles))

const selectedUnstagedPaths = computed(() =>
  unstagedFiles.value.map((f) => f.path).filter((path) => selectedFileSet.value.has(path)),
)
const selectedStagedPaths = computed(() =>
  stagedFiles.value.map((f) => f.path).filter((path) => selectedFileSet.value.has(path)),
)

const selectedInUnstaged = computed(() => selectedUnstagedPaths.value.length > 0)
const selectedInStaged = computed(() => selectedStagedPaths.value.length > 0)

const selectedUnstagedHasTracked = computed(() =>
  selectedUnstagedPaths.value.some((path) => !isUntrackedPath(path, props.statuses)),
)

function onFileClick(file: FileStatus, sectionFiles: FileStatus[], e: MouseEvent) {
  emit('selectFile', file.path, file.isStaged, {
    shiftKey: e.shiftKey,
    toggleKey: e.metaKey || e.ctrlKey,
    sectionPaths: sectionFiles.map((item) => item.path),
  })
}

function onFileMouseDown(e: MouseEvent) {
  if (e.shiftKey || e.metaKey || e.ctrlKey) e.preventDefault()
}

async function showInFolder(relPath: string, e: Event) {
  e.stopPropagation()
  if (!props.repoPath) { emit('revealError', '未打开仓库'); return }
  try {
    const abs = await join(props.repoPath, relPath)
    await revealItemInDir(abs)
  } catch (err) {
    emit('revealError', String(err))
  }
}

function subtreeBadge(path: string): string | null {
  return subtreePrefixForPath(path, props.subtrees)
}

function onWorkspaceScroll(event: Event) {
  workspaceScrollTop.value = (event.currentTarget as HTMLElement).scrollTop
}

function virtualWindow(files: FileStatus[], listEl: HTMLElement | null) {
  const viewportHeight = workspaceScrollRef.value?.clientHeight ?? 0
  const listTop = listEl?.offsetTop ?? 0
  const firstVisible = Math.floor((workspaceScrollTop.value - listTop) / FILE_ROW_HEIGHT)
  const visibleCount = Math.ceil(viewportHeight / FILE_ROW_HEIGHT)
  const start = Math.max(0, firstVisible - FILE_ROW_OVERSCAN)
  const end = Math.min(files.length, Math.max(0, firstVisible) + visibleCount + FILE_ROW_OVERSCAN)
  return {
    items: files.slice(start, end),
    top: start * FILE_ROW_HEIGHT,
    bottom: Math.max(0, (files.length - end) * FILE_ROW_HEIGHT),
  }
}

const visibleUnstaged = computed(() => virtualWindow(unstagedFiles.value, unstagedListRef.value))
const visibleStaged = computed(() => virtualWindow(stagedFiles.value, stagedListRef.value))
</script>

<template>
  <div ref="panelRef" class="h-full flex flex-col bg-[--bg-secondary] min-w-[320px]">
    <!-- Unstaged Changes -->
    <div ref="workspaceScrollRef" class="flex-1 overflow-y-auto min-h-0" @scroll="onWorkspaceScroll">
      <div class="flex items-center justify-between gap-2 px-2.5 py-2.5 text-xs text-[--text-secondary] uppercase tracking-wide bg-[--bg-tertiary] border-b border-[--border-color] sticky top-0 z-10 min-w-0">
        <span class="flex-shrink-0 whitespace-nowrap">Unstaged ({{ unstagedFiles.length }})</span>
        <div class="flex items-center gap-1 flex-nowrap flex-shrink-0 overflow-x-auto">
          <button
            v-if="selectedUnstagedHasTracked"
            class="flex items-center gap-1 px-2.5 py-2.5 rounded-[var(--radius)] text-xs text-[--diff-removed-text] hover:bg-[--diff-removed] transition-colors cursor-pointer disabled:opacity-30 whitespace-nowrap flex-shrink-0"
            :disabled="!selectedInUnstaged"
            @click="emit('revertFile', selectedUnstagedPaths, false)"
          >
            <Undo2 :size="12" />
            <span>Revert</span>
          </button>
          <button
            class="flex items-center gap-1 px-2.5 py-2.5 rounded-[var(--radius)] text-xs text-[--diff-removed-text] hover:bg-[--diff-removed] transition-colors cursor-pointer disabled:opacity-30 whitespace-nowrap flex-shrink-0"
            :disabled="!selectedInUnstaged"
            @click="emit('deleteFile', selectedUnstagedPaths, false)"
          >
            <Trash2 :size="12" />
            <span>Delete</span>
          </button>
          <button
            class="flex items-center gap-1 px-2.5 py-2.5 rounded-[var(--radius)] text-xs text-[--diff-added-text] hover:bg-[--diff-added] transition-colors cursor-pointer disabled:opacity-30 whitespace-nowrap flex-shrink-0"
            :disabled="!selectedInUnstaged"
            @click="emit('stageFile', selectedUnstagedPaths)"
          >
            <FilePlus :size="12" />
            <span>Stage</span>
          </button>
          <button
            v-if="unstagedFiles.length > 0"
            class="flex items-center gap-1 px-2.5 py-2.5 rounded-[var(--radius)] text-xs text-[--diff-added-text] hover:bg-[--diff-added] transition-colors cursor-pointer whitespace-nowrap flex-shrink-0"
            @click="emit('stageFile', unstagedFiles.map((f) => f.path))"
          >
            <FilePlus :size="12" />
            <span>全部 Stage</span>
          </button>
        </div>
      </div>
      <div v-if="statusLoading" class="flex items-center justify-center py-2.5 text-[--text-secondary]">
        <Loader2 :size="12" class="animate-spin mr-2" />
        <span class="text-xs">加载中...</span>
      </div>
      <div v-else-if="unstagedFiles.length === 0" class="px-2.5 py-2.5 text-xs text-[--text-secondary]">
        没有 Unstaged 变更
      </div>
      <div v-else ref="unstagedListRef">
        <div :style="{ height: `${visibleUnstaged.top}px` }" />
        <div
          v-for="file in visibleUnstaged.items"
          :key="file.path"
          class="flex h-12 items-center gap-1.5 px-2.5 py-2.5 text-xs border-b border-[--border-color] cursor-pointer transition-colors group select-none"
          :class="{ 'bg-green-900/20': selectedFileSet.has(file.path) }"
          @mousedown="onFileMouseDown"
          @click="onFileClick(file, unstagedFiles, $event)"
        >
          <button
            v-if="!isUntrackedFile(file)"
            class="flex-shrink-0 flex h-7 w-7 items-center justify-center rounded-[var(--radius)] bg-orange-700/60 hover:bg-orange-600 text-white transition-colors cursor-pointer"
            title="丢弃工作区变更"
            @click.stop="emit('revertFile', [file.path], false)"
          >
            <Undo2 :size="12" />
          </button>
          <button
            class="flex-shrink-0 flex h-7 w-7 items-center justify-center rounded-[var(--radius)] bg-red-800/70 hover:bg-red-700 text-white transition-colors cursor-pointer"
            title="删除文件"
            @click.stop="emit('deleteFile', [file.path], false)"
          >
            <Trash2 :size="12" />
          </button>
          <button
            class="flex-shrink-0 flex h-7 w-7 items-center justify-center rounded-[var(--radius)] bg-green-700/60 hover:bg-green-600 text-white transition-colors cursor-pointer"
            title="Stage 此文件"
            @click.stop="emit('stageFile', [file.path])"
          >
            <FilePlus :size="12" />
          </button>
          <span class="truncate flex-1 min-w-0 hover:text-[--accent] transition-colors font-mono-ui" :title="file.path">
            <span
              v-if="subtreeBadge(file.path)"
              class="inline-flex items-center rounded px-1 py-0.5 mr-1 text-[9px] font-semibold bg-sky-900/40 text-sky-300 leading-none align-middle"
              :title="`Subtree: ${subtreeBadge(file.path)}`"
            >ST</span><span
              v-if="conflictedFileSet.has(file.path)"
              class="inline-flex items-center rounded px-1 py-0.5 mr-1 text-[9px] font-semibold bg-amber-500/20 text-amber-300 leading-none align-middle"
            >冲突</span>{{ file.path }}
          </span>
          <button
            v-if="repoPath"
            type="button"
            class="flex-shrink-0 flex h-7 w-7 items-center justify-center rounded-[var(--radius)] text-[--text-secondary] hover:text-[--text-primary] hover:bg-[--bg-tertiary] opacity-0 pointer-events-none group-hover:opacity-100 group-hover:pointer-events-auto transition-opacity cursor-pointer"
            title="在文件夹中显示"
            @click.stop="showInFolder(file.path, $event)"
          >
            <FolderOpen :size="12" />
          </button>
          <span class="flex-shrink-0 text-[--diff-removed-text] font-mono-ui text-[10px]">{{ file.status }}</span>
        </div>
        <div :style="{ height: `${visibleUnstaged.bottom}px` }" />
      </div>

      <!-- Staged Changes -->
      <div class="flex items-center justify-between gap-2 px-2.5 py-2.5 text-xs text-[--text-secondary] uppercase tracking-wide bg-[--bg-tertiary] border-b border-[--border-color] sticky top-0 z-10 min-w-0">
        <span class="flex-shrink-0 whitespace-nowrap">Staged ({{ stagedFiles.length }})</span>
        <div v-if="stagedFiles.length > 0" class="flex items-center gap-1 flex-nowrap flex-shrink-0 overflow-x-auto">
          <button
            class="flex items-center gap-1 px-2.5 py-2.5 rounded-[var(--radius)] text-xs text-[--diff-removed-text] hover:bg-[--diff-removed] transition-colors cursor-pointer disabled:opacity-30 whitespace-nowrap flex-shrink-0"
            :disabled="!selectedInStaged"
            @click="emit('revertFile', selectedStagedPaths, true)"
          >
            <Undo2 :size="12" />
            <span>Revert</span>
          </button>
          <button
            class="flex items-center gap-1 px-2.5 py-2.5 rounded-[var(--radius)] text-xs text-[--diff-removed-text] hover:bg-[--diff-removed] transition-colors cursor-pointer disabled:opacity-30 whitespace-nowrap flex-shrink-0"
            :disabled="!selectedInStaged"
            @click="emit('deleteFile', selectedStagedPaths, true)"
          >
            <Trash2 :size="12" />
            <span>Delete</span>
          </button>
          <button
            class="flex items-center gap-1 px-2.5 py-2.5 rounded-[var(--radius)] text-xs text-[--diff-removed-text] hover:bg-[--diff-removed] transition-colors cursor-pointer disabled:opacity-30 whitespace-nowrap flex-shrink-0"
            :disabled="!selectedInStaged"
            @click="emit('unstageFile', selectedStagedPaths)"
          >
            <FileMinus :size="12" />
            <span>Unstage</span>
          </button>
          <button
            class="flex items-center gap-1 px-2.5 py-2.5 rounded-[var(--radius)] text-xs text-[--diff-removed-text] hover:bg-[--diff-removed] transition-colors cursor-pointer whitespace-nowrap flex-shrink-0"
            @click="emit('unstageFile', stagedFiles.map((f) => f.path))"
          >
            <FileMinus :size="12" />
            <span>全部 Unstage</span>
          </button>
        </div>
      </div>
      <div v-if="stagedFiles.length === 0" class="px-2.5 py-2.5 text-xs text-[--text-secondary]">
        没有 Staged 变更
      </div>
      <div v-else ref="stagedListRef">
        <div :style="{ height: `${visibleStaged.top}px` }" />
        <div
          v-for="file in visibleStaged.items"
          :key="file.path"
          class="flex h-12 items-center gap-1.5 px-2.5 py-2.5 text-xs border-b border-[--border-color] cursor-pointer transition-colors group select-none"
          :class="{ 'bg-red-900/20': selectedFileSet.has(file.path) }"
          @mousedown="onFileMouseDown"
          @click="onFileClick(file, stagedFiles, $event)"
        >
          <button
            class="flex-shrink-0 flex h-7 w-7 items-center justify-center rounded-[var(--radius)] bg-orange-700/60 hover:bg-orange-600 text-white transition-colors cursor-pointer"
            title="丢弃全部变更（含已 Stage）"
            @click.stop="emit('revertFile', [file.path], true)"
          >
            <Undo2 :size="12" />
          </button>
          <button
            class="flex-shrink-0 flex h-7 w-7 items-center justify-center rounded-[var(--radius)] bg-red-800/70 hover:bg-red-700 text-white transition-colors cursor-pointer"
            title="删除文件"
            @click.stop="emit('deleteFile', [file.path], true)"
          >
            <Trash2 :size="12" />
          </button>
          <button
            class="flex-shrink-0 flex h-7 w-7 items-center justify-center rounded-[var(--radius)] bg-red-700/60 hover:bg-red-600 text-white transition-colors cursor-pointer"
            title="Unstage 此文件"
            @click.stop="emit('unstageFile', [file.path])"
          >
            <FileMinus :size="12" />
          </button>
          <span class="truncate flex-1 min-w-0 hover:text-[--accent] transition-colors font-mono-ui" :title="file.path">
            <span
              v-if="subtreeBadge(file.path)"
              class="inline-flex items-center rounded px-1 py-0.5 mr-1 text-[9px] font-semibold bg-sky-900/40 text-sky-300 leading-none align-middle"
              :title="`Subtree: ${subtreeBadge(file.path)}`"
            >ST</span><span
              v-if="conflictedFileSet.has(file.path)"
              class="inline-flex items-center rounded px-1 py-0.5 mr-1 text-[9px] font-semibold bg-amber-500/20 text-amber-300 leading-none align-middle"
            >冲突</span>{{ file.path }}
          </span>
          <button
            v-if="repoPath"
            type="button"
            class="flex-shrink-0 flex h-7 w-7 items-center justify-center rounded-[var(--radius)] text-[--text-secondary] hover:text-[--text-primary] hover:bg-[--bg-tertiary] opacity-0 pointer-events-none group-hover:opacity-100 group-hover:pointer-events-auto transition-opacity cursor-pointer"
            title="在文件夹中显示"
            @click.stop="showInFolder(file.path, $event)"
          >
            <FolderOpen :size="12" />
          </button>
          <span class="flex-shrink-0 text-[--diff-added-text] font-mono-ui text-[10px]">{{ file.status }}</span>
        </div>
        <div :style="{ height: `${visibleStaged.bottom}px` }" />
      </div>
    </div>

    <!-- Unified Commit Area resize handle -->
    <div
      class="flex-shrink-0 h-[3px] cursor-ns-resize bg-[--border-color] hover:bg-[--accent] transition-colors"
      title="拖拽调整提交区高度"
      @mousedown="onCommitAreaResizeStart"
    />

    <!-- Unified Commit Area -->
    <div
      class="bg-[--bg-secondary] flex-shrink-0 flex flex-col overflow-hidden"
      :style="{ height: `${commitAreaHeight}px` }"
    >
      <div
        v-if="aiLoading"
        class="flex-shrink-0 flex items-center gap-2 px-2.5 py-1.5 border-b border-[--border-color] bg-[--bg-tertiary] text-[10px] text-[--text-secondary]"
      >
        <Loader2 :size="12" class="animate-spin text-[--accent] flex-shrink-0" />
        <span>正在加载 AI 设置与 Staged 差异…</span>
      </div>
      <template v-else-if="allModels.length > 0">
        <!-- Model selector + Generate button -->
        <div class="flex-shrink-0 flex items-center gap-2 px-2.5 py-1.5 border-b border-[--border-color] bg-[--bg-tertiary]">
          <Sparkles :size="14" class="text-[--accent] flex-shrink-0" />
          <div class="flex-1 min-w-0 flex items-stretch gap-1.5">
            <!-- Single model: show as label; multiple: show as dropdown -->
            <div v-if="allModels.length > 1" class="flex-1 min-w-0 flex" style="min-width: 0">
              <select
                v-model="selectedModelId"
                class="h-9 w-full min-w-0 box-border rounded-[var(--radius)] border border-[--border-color] bg-[--bg-secondary] px-2.5 py-0 text-xs leading-9 text-[--text-primary] outline-none transition-colors focus:border-[--accent] cursor-pointer"
                :disabled="generating"
              >
              <option value="" disabled>选择模型</option>
              <option
                v-for="m in allModels"
                :key="m.id"
                :value="m.id"
              >
                {{ m.name.length > 20 ? m.name.slice(0, 20) + '…' : m.name }}
              </option>
            </select>
            </div>
            <div
              v-else
              class="flex-1 min-w-0 flex items-center px-2.5 py-1.5 text-xs text-[--text-primary] font-mono-ui truncate"
              :title="`${allModels[0].name} (${allModels[0].provider.name})`"
            >
              {{ allModels[0].name }}
            </div>
            <button
              class="flex h-9 flex-shrink-0 items-center gap-1 rounded-[var(--radius)] bg-[--accent] px-3 text-xs text-white transition-colors hover:bg-[--accent-hover] disabled:cursor-not-allowed disabled:opacity-40 cursor-pointer whitespace-nowrap"
              :disabled="generating || aiLoading || !hasStagedFiles"
              @click="generateCommitMessage"
            >
              <Loader2 v-if="generating" :size="12" class="animate-spin" />
              <Sparkles v-else :size="12" />
              <span>{{ generating ? '生成中...' : '生成提交信息' }}</span>
            </button>
          </div>
        </div>

        <!-- Hint: no staged files -->
        <div
          v-if="!hasStagedFiles"
          class="flex-shrink-0 px-2.5 py-1 text-[10px] text-[--text-secondary] bg-[--bg-tertiary] border-b border-[--border-color]"
        >
          请先在文件列表中 Stage 文件后再使用 AI 生成提交信息
        </div>
      </template>
      <div
        v-else
        class="flex-shrink-0 flex items-center gap-2 px-2.5 py-1.5 border-b border-[--border-color] bg-[--bg-tertiary] text-xs text-[--text-secondary]"
      >
        <Sparkles :size="14" class="text-[--accent] flex-shrink-0 opacity-60" />
        <span class="flex-1 min-w-0 leading-relaxed">未配置模型与供应商时无法使用 AI 生成提交说明，请在设置中添加。</span>
        <button
          type="button"
          class="flex-shrink-0 flex items-center gap-1 px-2 py-1 rounded-[var(--radius)] text-[10px] bg-[--accent] text-white hover:bg-[--accent-hover] cursor-pointer"
          @click="emit('openSettings')"
        >
          <Settings :size="12" />
          打开设置
        </button>
      </div>

      <!-- Commit message input + button -->
      <div class="flex-1 min-h-0 flex flex-col p-2.5">
        <div v-if="aiError" class="mb-2 flex-shrink-0 flex items-start gap-1.5 p-2 rounded-[var(--radius)] bg-red-900/30 border border-red-800">
          <AlertCircle :size="13" class="text-red-400 flex-shrink-0 mt-0.5" />
          <span class="text-[11px] text-red-300 break-words flex-1">{{ aiError }}</span>
          <button class="flex-shrink-0 p-0.5 rounded text-red-400 hover:text-red-200 transition-colors cursor-pointer" @click="aiError = null">
            <span class="text-[11px]">✕</span>
          </button>
        </div>
        <textarea
          v-model="commitMessage"
          class="flex-1 min-h-[60px] w-full px-2.5 py-2.5 rounded-[var(--radius)] bg-[--bg-tertiary] border border-[--border-color] text-xs text-[--text-primary] placeholder-[--text-secondary] resize-none outline-none focus:border-[--accent] transition-colors font-mono-ui leading-relaxed"
          :placeholder="generating ? 'AI 正在生成提交信息...' : '提交信息...'"
          title="Cmd/Ctrl+Enter 提交，Shift+Cmd/Ctrl+Enter Amend"
          @keydown="handleCommitKeydown"
        />
        <div class="mt-2 flex flex-shrink-0 items-center gap-2 text-[10px] text-[--text-secondary]">
          <label class="flex min-w-0 flex-1 items-center gap-1.5 cursor-pointer select-none">
            <input
              v-model="amendLastCommit"
              type="checkbox"
              class="h-3.5 w-3.5 accent-[--accent]"
              :disabled="commitLoading || generating"
            />
            <span class="truncate">Amend last commit</span>
            <span class="hidden sm:inline text-[--text-secondary]">Shift+⌘/Ctrl+Enter</span>
          </label>
          <button
            type="button"
            class="flex flex-shrink-0 items-center gap-1 rounded-[var(--radius)] border border-[--border-color] px-2 py-1 text-[10px] text-[--text-secondary] transition-colors hover:border-orange-700/60 hover:bg-orange-900/20 hover:text-orange-300 disabled:cursor-not-allowed disabled:opacity-40 cursor-pointer"
            :disabled="commitLoading || generating"
            title="git reset --soft HEAD~1"
            @click="emit('softResetLastCommit')"
          >
            <RotateCcw :size="11" />
            Soft reset HEAD~1
          </button>
        </div>
        <button
          class="mt-2.5 flex-shrink-0 w-full flex items-center justify-center gap-1.5 px-2.5 py-2.5 rounded-[var(--radius)] text-xs bg-[--accent] text-white hover:bg-[--accent-hover] transition-colors disabled:opacity-40 disabled:cursor-not-allowed cursor-pointer"
          :disabled="!canSubmitCommit(commitMessage, commitLoading, generating)"
          @click="handleCommit()"
        >
          <Check v-if="commitSuccess && !commitLoading" :size="12" />
          <Loader2 v-else-if="commitLoading" :size="12" class="animate-spin" />
          <GitCommitVertical v-else :size="12" />
          <span>{{ commitSubmitLabel(amendLastCommit, commitLoading, commitSuccess) }}</span>
        </button>
      </div>
    </div>
  </div>
</template>
