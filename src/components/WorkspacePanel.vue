<script setup lang="ts">
import { ref, computed, onMounted, watch } from 'vue'
import { invoke } from '@tauri-apps/api/core'
import { fetch } from '@tauri-apps/plugin-http'
import { join } from '@tauri-apps/api/path'
import { revealItemInDir } from '@tauri-apps/plugin-opener'
import { FilePlus, FileMinus, FolderOpen, GitCommitVertical, Loader2, Sparkles, AlertCircle, RefreshCw } from 'lucide-vue-next'
import type { FileStatus, AppSettings, ProviderConfig, ModelConfig } from '../types'

const props = defineProps<{
  statuses: FileStatus[]
  selectedFile: string | null
  commitLoading: boolean
  statusLoading: boolean
  repoPath: string | null
  settingsRevision: number
}>()

const emit = defineEmits<{
  stageFile: [path: string]
  unstageFile: [path: string]
  selectFile: [path: string, isStaged: boolean]
  commit: [message: string]
  revealError: [message: string]
}>()

// === Manual commit state ===
const commitMessage = ref('')

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

  try {
    const model = selectedModel.value
    // Re-fetch staged diff to ensure it's up-to-date
    const diff = await invoke<string>('get_staged_diff')
    stagedDiff.value = diff
    const prompt = buildPrompt(diff)

    if (model.provider.type === 'openai') {
      await streamOpenAI(model.provider, model.name, prompt)
    } else {
      await streamAnthropic(model.provider, model.name, prompt)
    }
  } catch (e: any) {
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

async function streamOpenAI(provider: ProviderConfig, model: string, prompt: string): Promise<void> {
  const baseUrl = provider.baseUrl.replace(/\/+$/, '')
  const url = baseUrl.includes('/chat/completions') ? baseUrl : `${baseUrl}/chat/completions`

  const resp = await fetch(url, {
    method: 'POST',
    headers: {
      'Content-Type': 'application/json',
      'Authorization': `Bearer ${provider.apiKey}`,
    },
    body: JSON.stringify({
      model,
      messages: [{ role: 'user', content: prompt }],
      max_tokens: 2048,
      temperature: 0.3,
      stream: true,
    }),
  })

  if (!resp.ok) {
    const text = await resp.text()
    throw new Error(`OpenAI API error (${resp.status}): ${text}`)
  }

  const reader = resp.body?.getReader()
  if (!reader) throw new Error('Streaming not supported by HTTP client')
  const decoder = new TextDecoder()
  let buffer = ''

  while (true) {
    const { done, value } = await reader.read()
    if (done) break
    buffer += decoder.decode(value, { stream: true })

    const lines = buffer.split('\n')
    buffer = lines.pop() || ''

    for (const line of lines) {
      const trimmed = line.trim()
      if (!trimmed || trimmed === 'data: [DONE]') continue
      if (!trimmed.startsWith('data: ')) continue

      try {
        const data = JSON.parse(trimmed.slice(6))
        const content = data?.choices?.[0]?.delta?.content
        if (content) commitMessage.value += content
      } catch { /* skip malformed JSON lines */ }
    }
  }
}

async function streamAnthropic(provider: ProviderConfig, model: string, prompt: string): Promise<void> {
  const baseUrl = provider.baseUrl.replace(/\/+$/, '')
  const url = baseUrl.includes('/v1/messages') ? baseUrl : `${baseUrl}/v1/messages`

  const resp = await fetch(url, {
    method: 'POST',
    headers: {
      'Content-Type': 'application/json',
      'x-api-key': provider.apiKey,
      'anthropic-version': '2023-06-01',
    },
    body: JSON.stringify({
      model,
      max_tokens: 2048,
      messages: [{ role: 'user', content: prompt }],
      stream: true,
    }),
  })

  if (!resp.ok) {
    const text = await resp.text()
    throw new Error(`Anthropic API error (${resp.status}): ${text}`)
  }

  const reader = resp.body?.getReader()
  if (!reader) throw new Error('Streaming not supported by HTTP client')
  const decoder = new TextDecoder()
  let buffer = ''
  let currentEvent = ''

  while (true) {
    const { done, value } = await reader.read()
    if (done) break
    buffer += decoder.decode(value, { stream: true })

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
        if (currentEvent === 'content_block_delta' && data?.delta?.text) {
          commitMessage.value += data.delta.text
        } else if (currentEvent === 'content_block_start' && data?.content_block?.text) {
          commitMessage.value += data.content_block.text
        }
      } catch { /* skip malformed JSON */ }
    }
  }
}

// === File list helpers ===
const unstagedFiles = (statuses: FileStatus[]) => statuses.filter((f) => !f.isStaged)
const stagedFiles = (statuses: FileStatus[]) => statuses.filter((f) => f.isStaged)

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
            class="flex items-center gap-1 px-2.5 py-2.5 rounded-[var(--radius)] text-xs text-[--diff-added-text] hover:bg-[--diff-added] transition-colors cursor-pointer disabled:opacity-30"
            :disabled="!props.selectedFile"
            @click="emit('stageFile', props.selectedFile!)"
          >
            <FilePlus :size="12" />
            <span>暂存</span>
          </button>
          <button
            v-if="unstagedFiles(statuses).length > 0"
            class="flex items-center gap-1 px-2.5 py-2.5 rounded-[var(--radius)] text-xs text-[--diff-added-text] hover:bg-[--diff-added] transition-colors cursor-pointer"
            @click="unstagedFiles(statuses).forEach(f => emit('stageFile', f.path))"
          >
            <FilePlus :size="12" />
            <span>全部暂存</span>
          </button>
        </div>
      </div>
      <div v-if="statusLoading" class="flex items-center justify-center py-2.5 text-[--text-secondary]">
        <Loader2 :size="12" class="animate-spin mr-2" />
        <span class="text-xs">加载中...</span>
      </div>
      <div v-else-if="unstagedFiles(statuses).length === 0" class="px-2.5 py-2.5 text-xs text-[--text-secondary]">
        没有未暂存的变更
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
            class="flex-shrink-0 flex h-7 w-7 items-center justify-center rounded-[var(--radius)] bg-green-700/60 hover:bg-green-600 text-white transition-colors cursor-pointer"
            title="暂存此文件"
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
        <button
          v-if="stagedFiles(statuses).length > 0"
          class="text-[10px] px-2.5 py-2.5 rounded-[var(--radius)] text-[--diff-removed-text] hover:bg-[--diff-removed] transition-colors cursor-pointer"
          @click="stagedFiles(statuses).forEach(f => emit('unstageFile', f.path))"
        >全部撤销</button>
      </div>
      <div v-if="stagedFiles(statuses).length === 0" class="px-2.5 py-2.5 text-xs text-[--text-secondary]">
        没有已暂存的变更
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
            class="flex-shrink-0 flex h-7 w-7 items-center justify-center rounded-[var(--radius)] bg-red-700/60 hover:bg-red-600 text-white transition-colors cursor-pointer"
            title="撤销暂存"
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
      <!-- AI section: only when models are configured -->
      <template v-if="!aiLoading && allModels.length > 0">
        <!-- Model selector + Generate button -->
        <div class="flex items-center gap-2 px-2.5 py-1.5 border-b border-[--border-color] bg-[--bg-tertiary]">
          <Sparkles :size="14" class="text-[--accent] flex-shrink-0" />
          <div class="flex-1 flex items-stretch gap-1.5">
            <!-- Single model: show as label; multiple: show as dropdown -->
            <select
              v-if="allModels.length > 1"
              v-model="selectedModelId"
              class="flex-1 min-w-0 px-2.5 py-1.5 rounded-[var(--radius)] bg-[--bg-secondary] border border-[--border-color] text-xs text-[--text-primary] outline-none focus:border-[--accent] transition-colors cursor-pointer"
              :disabled="generating"
            >
              <option value="" disabled>选择模型</option>
              <option
                v-for="m in allModels"
                :key="m.id"
                :value="m.id"
              >
                {{ m.name }} ({{ m.provider.name }} / {{ m.provider.type === 'openai' ? 'OpenAI' : 'Anthropic' }})
              </option>
            </select>
            <div
              v-else
              class="flex-1 flex items-center px-2.5 py-1.5 text-xs text-[--text-primary] font-mono-ui truncate"
            >
              {{ allModels[0].name }} ({{ allModels[0].provider.name }})
            </div>
            <button
              class="flex items-center gap-1 px-3 py-1.5 rounded-[var(--radius)] text-xs bg-[--accent] text-white hover:bg-[--accent-hover] transition-colors disabled:opacity-40 disabled:cursor-not-allowed cursor-pointer whitespace-nowrap"
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
          请先在文件列表中暂存文件后再使用 AI 生成提交信息
        </div>
      </template>

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
          <GitCommitVertical :size="12" />
          <span>{{ commitLoading ? '提交中...' : 'Commit' }}</span>
        </button>
      </div>
    </div>
  </div>
</template>
