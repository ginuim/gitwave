<script setup lang="ts">
import { ref, computed, onMounted, watch } from 'vue'
import { invoke } from '@tauri-apps/api/core'
import { fetch } from '@tauri-apps/plugin-http'
import { Sparkles, GitCommitVertical, Loader2, RefreshCw, AlertCircle } from 'lucide-vue-next'
import type { AppSettings, ProviderConfig, ModelConfig } from '../types'

const props = defineProps<{
  settingsRevision: number
}>()

const emit = defineEmits<{
  commit: [message: string]
}>()

// --- State ---
const settings = ref<AppSettings | null>(null)
const stagedDiff = ref<string>('')
const generatedMessage = ref('')
const editedMessage = ref('')
const generating = ref(false)
const loading = ref(true)
const error = ref<string | null>(null)
const selectedModelId = ref<string>('')
const clarificationNeeded = ref(false)
const clarificationQuestion = ref('')
const clarificationAnswer = ref('')
const clarifying = ref(false)

// --- Computed ---
const allModels = computed<(ModelConfig & { provider: ProviderConfig })[]>(() => {
  if (!settings.value) return []
  const result: (ModelConfig & { provider: ProviderConfig })[] = []
  for (const prov of settings.value.providers) {
    for (const model of settings.value.models.filter(m => m.providerId === prov.id)) {
      result.push({ ...model, provider: prov })
    }
  }
  return result
})

const selectedModel = computed(() => {
  return allModels.value.find(m => m.id === selectedModelId.value) || null
})

const defaultModel = computed(() => {
  return allModels.value.find(m => m.isDefault) || allModels.value[0] || null
})

const hasStagedFiles = computed(() => {
  // stagedDiff will be non-empty when there's actual staged content
  return stagedDiff.value.trim().length > 0
})

const canGenerate = computed(() => {
  return hasStagedFiles.value && selectedModel.value !== null && !generating.value
})

const commitPrompt = computed(() => {
  return settings.value?.prompts?.commitPrompt || ''
})

// --- Lifecycle ---
onMounted(async () => {
  await loadData()
})

// Reload when settings revision changes
watch(() => props.settingsRevision, () => {
  // Only reload if not actively generating
  if (!generating.value) {
    // Keep the current staged diff, don't refetch it — only settings changed
    reloadSettings()
  }
})

async function reloadSettings() {
  try {
    const s = await invoke<AppSettings>('load_settings')
    settings.value = s
    // Re-evaluate selected model
    if (selectedModelId.value && !allModels.value.find(m => m.id === selectedModelId.value)) {
      const d = defaultModel.value
      if (d) selectedModelId.value = d.id
    } else if (allModels.value.length > 0 && !selectedModelId.value) {
      const d = defaultModel.value
      if (d) selectedModelId.value = d.id
    }
  } catch (_) {
    // silent
  }
}

async function loadData() {
  loading.value = true
  error.value = null
  try {
    const [s, diff] = await Promise.all([
      invoke<AppSettings>('load_settings'),
      invoke<string>('get_staged_diff'),
    ])
    settings.value = s
    stagedDiff.value = diff

    // Auto-select default model
    if (!selectedModelId.value || !allModels.value.find(m => m.id === selectedModelId.value)) {
      if (defaultModel.value) {
        selectedModelId.value = defaultModel.value.id
      } else if (allModels.value.length > 0) {
        selectedModelId.value = allModels.value[0].id
      }
    }
  } catch (e: any) {
    error.value = String(e)
  } finally {
    loading.value = false
  }
}

// --- LLM Call ---
async function generateCommitMessage() {
  if (!canGenerate.value || !selectedModel.value) return

  generating.value = true
  error.value = null
  clarificationNeeded.value = false
  clarificationQuestion.value = ''
  clarificationAnswer.value = ''

  try {
    const model = selectedModel.value
    const diff = stagedDiff.value
    const prompt = buildPrompt(diff)

    let result: string
    if (model.provider.type === 'openai') {
      result = await callOpenAI(model.provider, model.name, prompt)
    } else {
      result = await callAnthropic(model.provider, model.name, prompt)
    }

    generatedMessage.value = result.trim()
    editedMessage.value = generatedMessage.value
  } catch (e: any) {
    error.value = String(e)
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

async function callOpenAI(provider: ProviderConfig, model: string, prompt: string): Promise<string> {
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
    }),
  })

  const rawText = await resp.text()

  if (!resp.ok) {
    throw new Error(`OpenAI API error (${resp.status}): ${rawText}`)
  }

  let data: any
  try {
    data = JSON.parse(rawText)
  } catch {
    throw new Error(`OpenAI returned non-JSON response: ${rawText.slice(0, 500)}`)
  }

  const content = data?.choices?.[0]?.message?.content
  if (content) return content

  // Some OpenAI-compatible might return differently
  if (data?.response) return data.response
  if (data?.text) return data.text

  throw new Error(`OpenAI returned unexpected format: ${JSON.stringify(data).slice(0, 500)}`)
}

async function callAnthropic(provider: ProviderConfig, model: string, prompt: string): Promise<string> {
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
    }),
  })

  // Read raw text first for debugging
  const rawText = await resp.text()

  if (!resp.ok) {
    throw new Error(`Anthropic API error (${resp.status}): ${rawText}`)
  }

  let data: any
  try {
    data = JSON.parse(rawText)
  } catch {
    throw new Error(`Anthropic returned non-JSON response: ${rawText.slice(0, 500)}`)
  }

  // Standard Anthropic format: content[] may contain multiple blocks (text, thinking, tool_use...)
  if (Array.isArray(data?.content)) {
    for (const block of data.content) {
      if (block?.type === 'text' && block?.text) return block.text
    }
  }

  // Some API returns content as a plain string
  if (data?.content && typeof data.content === 'string') return data.content

  // Some return in choices format like OpenAI
  const openaiContent = data?.choices?.[0]?.message?.content
  if (openaiContent) return openaiContent

  // Fallback: try to find any text field
  const anyText = data?.completion || data?.text || data?.response
  if (anyText) return anyText

  throw new Error(`Anthropic returned unexpected format: ${JSON.stringify(data).slice(0, 500)}`)
}

// --- Actions ---
function handleCommit() {
  if (!editedMessage.value.trim()) return
  emit('commit', editedMessage.value.trim())
}

function handleRegenerate() {
  generateCommitMessage()
}

function handleEdit(msg: string) {
  editedMessage.value = msg
}

// --- Clarification ---
function askClarification(question: string) {
  clarificationNeeded.value = true
  clarificationQuestion.value = question
  clarificationAnswer.value = ''
}

function submitClarification() {
  if (!clarificationAnswer.value.trim()) return
  clarifying.value = true
  // Re-generate with the additional context
  // For now, append the answer to the prompt and retry
  generatedMessage.value += `\n\n[用户澄清: ${clarificationAnswer.value.trim()}]`
  editedMessage.value = generatedMessage.value
  clarificationNeeded.value = false
  clarificationAnswer.value = ''
  clarifying.value = false
}
</script>

<template>
  <div class="flex flex-col h-full">
    <!-- Model selector row -->
    <div class="flex items-center gap-2 px-2.5 py-1.5 border-b border-[--border-color] bg-[--bg-secondary]">
      <div class="flex-1 flex items-center gap-1.5">
        <Sparkles :size="13" class="text-[--accent] flex-shrink-0" />
        <select
          v-model="selectedModelId"
          class="flex-1 px-2 py-1.5 rounded-[var(--radius)] bg-[--bg-tertiary] border border-[--border-color] text-xs text-[--text-primary] outline-none focus:border-[--accent] transition-colors"
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
      </div>
      <button
        class="flex items-center gap-1 px-2.5 py-1.5 rounded-[var(--radius)] text-xs bg-[--accent] text-white hover:bg-[--accent-hover] transition-colors disabled:opacity-40 disabled:cursor-not-allowed cursor-pointer whitespace-nowrap"
        :disabled="!canGenerate"
        @click="generateCommitMessage"
      >
        <Sparkles :size="12" />
        <span>{{ generating ? '生成中...' : '生成提交信息' }}</span>
      </button>
    </div>

    <!-- Content area -->
    <div class="flex-1 overflow-y-auto p-2.5">
      <!-- Loading -->
      <div v-if="loading" class="flex items-center justify-center h-full text-xs text-[--text-secondary]">
        <Loader2 :size="14" class="animate-spin mr-2" />
        <span>加载中...</span>
      </div>

      <!-- Error -->
      <div v-else-if="error" class="flex flex-col items-center justify-center h-full text-xs gap-2">
        <AlertCircle :size="20" class="text-red-400" />
        <div class="text-red-400 text-center max-w-xs break-words">{{ error }}</div>
        <button
          class="flex items-center gap-1 px-2.5 py-1.5 rounded-[var(--radius)] text-xs bg-[--bg-tertiary] text-[--text-secondary] hover:text-[--text-primary] hover:bg-[--bg-secondary] transition-colors cursor-pointer"
          @click="loadData"
        >
          <RefreshCw :size="12" />
          <span>重试</span>
        </button>
      </div>

      <!-- No staged files -->
      <div v-else-if="!hasStagedFiles" class="flex items-center justify-center h-full text-xs text-[--text-secondary]">
        没有已暂存的变更，请先在文件列表暂存文件后再使用 AI 提交
      </div>

      <!-- AI generated message -->
      <div v-else class="flex flex-col h-full gap-2">
        <!-- Clarification dialog -->
        <div
          v-if="clarificationNeeded"
          class="p-2.5 rounded-[var(--radius)] bg-[--bg-tertiary] border border-[--border-color]"
        >
          <div class="text-xs text-[--text-primary] font-medium mb-1.5">AI 需要澄清</div>
          <div class="text-xs text-[--text-secondary] mb-2">{{ clarificationQuestion }}</div>
          <textarea
            v-model="clarificationAnswer"
            class="w-full px-2 py-1.5 rounded-[var(--radius)] bg-[--bg-primary] border border-[--border-color] text-xs text-[--text-primary] outline-none focus:border-[--accent] transition-colors resize-none"
            rows="2"
            placeholder="请输入补充说明..."
          ></textarea>
          <button
            class="mt-1.5 px-2.5 py-1.5 rounded-[var(--radius)] text-xs bg-[--accent] text-white hover:bg-[--accent-hover] transition-colors disabled:opacity-40 cursor-pointer"
            :disabled="!clarificationAnswer.trim() || clarifying"
            @click="submitClarification"
          >
            {{ clarifying ? '提交中...' : '确认' }}
          </button>
        </div>

        <!-- Generated message -->
        <div v-if="generatedMessage" class="flex flex-col flex-1 min-h-0">
          <div class="flex items-center justify-between mb-1">
            <span class="text-[11px] text-[--text-secondary]">生成的提交信息</span>
            <button
              class="flex items-center gap-1 px-2 py-1 rounded-[var(--radius)] text-[10px] text-[--text-secondary] hover:text-[--text-primary] hover:bg-[--bg-tertiary] transition-colors cursor-pointer"
              :disabled="generating"
              @click="handleRegenerate"
            >
              <RefreshCw :size="11" :class="{ 'animate-spin': generating }" />
              <span>重新生成</span>
            </button>
          </div>
          <textarea
            v-model="editedMessage"
            class="flex-1 w-full px-2.5 py-2 rounded-[var(--radius)] bg-[--bg-primary] border border-[--border-color] text-xs text-[--text-primary] outline-none focus:border-[--accent] transition-colors font-mono-ui resize-none leading-relaxed"
            @input="handleEdit(editedMessage)"
          ></textarea>

          <!-- Diff preview toggle -->
          <details class="mt-1.5">
            <summary class="text-[10px] text-[--text-secondary] cursor-pointer hover:text-[--text-primary] transition-colors">
              查看 Diff
            </summary>
            <pre class="mt-1 p-2 rounded-[var(--radius)] bg-[--bg-primary] border border-[--border-color] text-[10px] text-[--text-secondary] font-mono-ui overflow-x-auto max-h-[120px] overflow-y-auto">{{ stagedDiff }}</pre>
          </details>
        </div>

        <!-- Generating indicator -->
        <div v-else-if="generating" class="flex items-center justify-center flex-1 text-xs text-[--text-secondary]">
          <Loader2 :size="14" class="animate-spin mr-2" />
          <span>AI 正在生成提交信息...</span>
        </div>

        <!-- Initial state -->
        <div v-else class="flex items-center justify-center flex-1 text-xs text-[--text-secondary]">
          点击上方「生成提交信息」按钮，AI 将根据已暂存的变更生成规范的提交信息
        </div>
      </div>
    </div>

    <!-- Commit button -->
    <div class="border-t border-[--border-color] p-2.5 bg-[--bg-secondary] flex-shrink-0">
      <button
        class="w-full flex items-center justify-center gap-1.5 px-2.5 py-2.5 rounded-[var(--radius)] text-xs bg-[--accent] text-white hover:bg-[--accent-hover] transition-colors disabled:opacity-40 disabled:cursor-not-allowed cursor-pointer"
        :disabled="!editedMessage.trim() || generating"
        @click="handleCommit"
      >
        <GitCommitVertical :size="12" />
        <span>Commit</span>
      </button>
    </div>
  </div>
</template>
