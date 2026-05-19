<script setup lang="ts">
import { ref, computed, onMounted, watch } from 'vue'
import { invoke } from '@tauri-apps/api/core'
import { fetch } from '@tauri-apps/plugin-http'
import { join } from '@tauri-apps/api/path'
import { revealItemInDir } from '@tauri-apps/plugin-opener'
import { FilePlus, FileMinus, FolderOpen, GitCommitVertical, Loader2, Sparkles, AlertCircle, Check, Settings, Undo2 } from 'lucide-vue-next'
import type { FileStatus, AppSettings, ProviderConfig, ModelConfig } from '../types'
import { isUntrackedFile, isUntrackedPath } from '../utils/gitStatus'

const props = defineProps<{
  statuses: FileStatus[]
  selectedFile: string | null
  commitLoading: boolean
  /** 父组件在提交成功后递增，用于显示「已提交」短提示 */
  commitSuccessTick: number
  statusLoading: boolean
  repoPath: string | null
  settingsRevision: number
}>()

const emit = defineEmits<{
  stageFile: [path: string]
  unstageFile: [path: string]
  revertFile: [path: string, isStaged: boolean]
  selectFile: [path: string, isStaged: boolean]
  commit: [message: string]
  revealError: [message: string]
  openSettings: []
}>()

// === Manual commit state ===
const commitMessage = ref('')
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

function handleCommit() {
  if (!commitMessage.value.trim()) return
  emit('commit', commitMessage.value)
  commitMessage.value = ''
}

// === AI commit state ===
const aiSettings = ref<AppSettings | null>(null)
const stagedDiff = ref<string>('')
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

// Refresh staged diff whenever the staged file list changes
watch(
  () => props.statuses.filter(f => f.isStaged).map(f => f.path).join('\0'),
  async () => {
    if (!props.repoPath || generating.value) return
    try {
      stagedDiff.value = await invoke<string>('get_staged_diff')
    } catch (_) { /* silent */ }
  },
)

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
    const [s, diff] = await Promise.all([
      invoke<AppSettings>('load_settings'),
      invoke<string>('get_staged_diff'),
    ])
    aiSettings.value = s
    stagedDiff.value = diff
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

    const prompt = buildPrompt(stagedDiff.value)
    log(`prompt built — ${prompt.length} chars, diff=${stagedDiff.value.length} chars`)

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

function buildPrompt(diff: string): string {
  const prompt = commitPrompt.value || ''
  return `You are a git commit message generator.

${prompt ? `## Commit Convention\n${prompt}\n\n` : ''}## Staged Changes (diff)
\`\`\`diff
${diff}
\`\`\`

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
const unstagedFiles = (statuses: FileStatus[]) => statuses.filter((f) => !f.isStaged)
const stagedFiles = (statuses: FileStatus[]) => statuses.filter((f) => f.isStaged)

const selectedInUnstaged = computed(
  () =>
  !!props.selectedFile &&
  unstagedFiles(props.statuses).some((f) => f.path === props.selectedFile),
)
const selectedInStaged = computed(
  () =>
  !!props.selectedFile &&
  stagedFiles(props.statuses).some((f) => f.path === props.selectedFile),
)

const selectedIsUntracked = computed(
  () => props.selectedFile != null && isUntrackedPath(props.selectedFile, props.statuses),
)

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
</script>

<template>
  <div class="h-full flex flex-col bg-[--bg-secondary]">
    <!-- Unstaged Changes -->
    <div class="flex-1 overflow-y-auto min-h-0">
      <div class="flex items-center justify-between px-2.5 py-2.5 text-xs text-[--text-secondary] uppercase tracking-wide bg-[--bg-tertiary] border-b border-[--border-color] sticky top-0 z-10">
        <span>Unstaged ({{ unstagedFiles(statuses).length }})</span>
        <div class="flex items-center gap-1">
          <button
            v-if="!selectedIsUntracked"
            class="flex items-center gap-1 px-2.5 py-2.5 rounded-[var(--radius)] text-xs text-[--diff-removed-text] hover:bg-[--diff-removed] transition-colors cursor-pointer disabled:opacity-30"
            :disabled="!selectedInUnstaged"
            @click="emit('revertFile', props.selectedFile!, false)"
          >
            <Undo2 :size="12" />
            <span>Revert</span>
          </button>
          <button
            class="flex items-center gap-1 px-2.5 py-2.5 rounded-[var(--radius)] text-xs text-[--diff-added-text] hover:bg-[--diff-added] transition-colors cursor-pointer disabled:opacity-30"
            :disabled="!selectedInUnstaged"
            @click="emit('stageFile', props.selectedFile!)"
          >
            <FilePlus :size="12" />
            <span>Stage</span>
          </button>
          <button
            v-if="unstagedFiles(statuses).length > 0"
            class="flex items-center gap-1 px-2.5 py-2.5 rounded-[var(--radius)] text-xs text-[--diff-added-text] hover:bg-[--diff-added] transition-colors cursor-pointer"
            @click="unstagedFiles(statuses).forEach(f => emit('stageFile', f.path))"
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
      <div v-else-if="unstagedFiles(statuses).length === 0" class="px-2.5 py-2.5 text-xs text-[--text-secondary]">
        没有 Unstaged 变更
      </div>
      <div v-else>
        <div
          v-for="file in unstagedFiles(statuses)"
          :key="file.path"
          class="flex items-center gap-1.5 px-2.5 py-2.5 text-xs border-b border-[--border-color] cursor-pointer transition-colors group"
          :class="{ 'bg-green-900/20': selectedFile === file.path }"
          @click="emit('selectFile', file.path, file.isStaged)"
        >
          <button
            v-if="!isUntrackedFile(file)"
            class="flex-shrink-0 flex h-7 w-7 items-center justify-center rounded-[var(--radius)] bg-orange-700/60 hover:bg-orange-600 text-white transition-colors cursor-pointer"
            title="丢弃工作区变更"
            @click.stop="emit('revertFile', file.path, false)"
          >
            <Undo2 :size="12" />
          </button>
          <button
            class="flex-shrink-0 flex h-7 w-7 items-center justify-center rounded-[var(--radius)] bg-green-700/60 hover:bg-green-600 text-white transition-colors cursor-pointer"
            title="Stage 此文件"
            @click.stop="emit('stageFile', file.path)"
          >
            <FilePlus :size="12" />
          </button>
          <span class="truncate flex-1 hover:text-[--accent] transition-colors font-mono-ui">{{ file.path }}</span>
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
      </div>

      <!-- Staged Changes -->
      <div class="flex items-center justify-between px-2.5 py-2.5 text-xs text-[--text-secondary] uppercase tracking-wide bg-[--bg-tertiary] border-b border-[--border-color] sticky top-0 z-10">
        <span>Staged ({{ stagedFiles(statuses).length }})</span>
        <div v-if="stagedFiles(statuses).length > 0" class="flex items-center gap-1">
          <button
            class="flex items-center gap-1 px-2.5 py-2.5 rounded-[var(--radius)] text-xs text-[--diff-removed-text] hover:bg-[--diff-removed] transition-colors cursor-pointer disabled:opacity-30"
            :disabled="!selectedInStaged"
            @click="emit('revertFile', props.selectedFile!, true)"
          >
            <Undo2 :size="12" />
            <span>Revert</span>
          </button>
          <button
            class="flex items-center gap-1 px-2.5 py-2.5 rounded-[var(--radius)] text-xs text-[--diff-removed-text] hover:bg-[--diff-removed] transition-colors cursor-pointer disabled:opacity-30"
            :disabled="!selectedInStaged"
            @click="emit('unstageFile', props.selectedFile!)"
          >
            <FileMinus :size="12" />
            <span>Unstage</span>
          </button>
          <button
            class="flex items-center gap-1 px-2.5 py-2.5 rounded-[var(--radius)] text-xs text-[--diff-removed-text] hover:bg-[--diff-removed] transition-colors cursor-pointer"
            @click="stagedFiles(statuses).forEach(f => emit('unstageFile', f.path))"
          >
            <FileMinus :size="12" />
            <span>全部 Unstage</span>
          </button>
        </div>
      </div>
      <div v-if="stagedFiles(statuses).length === 0" class="px-2.5 py-2.5 text-xs text-[--text-secondary]">
        没有 Staged 变更
      </div>
      <div v-else>
        <div
          v-for="file in stagedFiles(statuses)"
          :key="file.path"
          class="flex items-center gap-1.5 px-2.5 py-2.5 text-xs border-b border-[--border-color] cursor-pointer transition-colors group"
          :class="{ 'bg-red-900/20': selectedFile === file.path }"
          @click="emit('selectFile', file.path, file.isStaged)"
        >
          <button
            class="flex-shrink-0 flex h-7 w-7 items-center justify-center rounded-[var(--radius)] bg-orange-700/60 hover:bg-orange-600 text-white transition-colors cursor-pointer"
            title="丢弃全部变更（含已 Stage）"
            @click.stop="emit('revertFile', file.path, true)"
          >
            <Undo2 :size="12" />
          </button>
          <button
            class="flex-shrink-0 flex h-7 w-7 items-center justify-center rounded-[var(--radius)] bg-red-700/60 hover:bg-red-600 text-white transition-colors cursor-pointer"
            title="Unstage 此文件"
            @click.stop="emit('unstageFile', file.path)"
          >
            <FileMinus :size="12" />
          </button>
          <span class="truncate flex-1 hover:text-[--accent] transition-colors font-mono-ui">{{ file.path }}</span>
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
      </div>
    </div>

    <!-- Unified Commit Area -->
    <div class="border-t border-[--border-color] bg-[--bg-secondary] flex-shrink-0">
      <div
        v-if="aiLoading"
        class="flex items-center gap-2 px-2.5 py-1.5 border-b border-[--border-color] bg-[--bg-tertiary] text-[10px] text-[--text-secondary]"
      >
        <Loader2 :size="12" class="animate-spin text-[--accent] flex-shrink-0" />
        <span>正在加载 AI 设置与 Staged 差异…</span>
      </div>
      <template v-else-if="allModels.length > 0">
        <!-- Model selector + Generate button -->
        <div class="flex items-center gap-2 px-2.5 py-1.5 border-b border-[--border-color] bg-[--bg-tertiary]">
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
          class="px-2.5 py-1 text-[10px] text-[--text-secondary] bg-[--bg-tertiary] border-b border-[--border-color]"
        >
          请先在文件列表中 Stage 文件后再使用 AI 生成提交信息
        </div>
      </template>
      <div
        v-else
        class="flex items-center gap-2 px-2.5 py-1.5 border-b border-[--border-color] bg-[--bg-tertiary] text-xs text-[--text-secondary]"
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
      <div class="p-2.5">
        <div v-if="aiError" class="mb-2 flex items-start gap-1.5 p-2 rounded-[var(--radius)] bg-red-900/30 border border-red-800">
          <AlertCircle :size="13" class="text-red-400 flex-shrink-0 mt-0.5" />
          <span class="text-[11px] text-red-300 break-words flex-1">{{ aiError }}</span>
          <button class="flex-shrink-0 p-0.5 rounded text-red-400 hover:text-red-200 transition-colors cursor-pointer" @click="aiError = null">
            <span class="text-[11px]">✕</span>
          </button>
        </div>
        <textarea
          v-model="commitMessage"
          class="w-full px-2.5 py-2.5 rounded-[var(--radius)] bg-[--bg-tertiary] border border-[--border-color] text-xs text-[--text-primary] placeholder-[--text-secondary] resize-none outline-none focus:border-[--accent] transition-colors font-mono-ui leading-relaxed"
          rows="3"
          :placeholder="generating ? 'AI 正在生成提交信息...' : '提交信息...'"
          @keydown.meta.enter="handleCommit"
          @keydown.ctrl.enter="handleCommit"
        />
        <button
          class="mt-2.5 w-full flex items-center justify-center gap-1.5 px-2.5 py-2.5 rounded-[var(--radius)] text-xs bg-[--accent] text-white hover:bg-[--accent-hover] transition-colors disabled:opacity-40 disabled:cursor-not-allowed cursor-pointer"
          :disabled="!commitMessage.trim() || commitLoading || generating"
          @click="handleCommit"
        >
          <Check v-if="commitSuccess && !commitLoading" :size="12" />
          <Loader2 v-else-if="commitLoading" :size="12" class="animate-spin" />
          <GitCommitVertical v-else :size="12" />
          <span>{{ commitLoading ? '提交中...' : commitSuccess ? '已提交' : 'Commit' }}</span>
        </button>
      </div>
    </div>
  </div>
</template>
