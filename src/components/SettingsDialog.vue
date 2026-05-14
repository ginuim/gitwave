<script setup lang="ts">
import { ref, reactive, watch, computed } from 'vue'
import { invoke } from '@tauri-apps/api/core'
import { X, Plus, Trash2, Settings, Globe, Bot, FileText, Star } from 'lucide-vue-next'

export interface ProviderConfig {
  id: string
  name: string
  type: 'openai' | 'anthropic'
  baseUrl: string
  apiKey: string
  isDefault: boolean
}

export interface ModelConfig {
  id: string
  providerId: string
  name: string
  isDefault: boolean
}

export interface AppSettings {
  general: {
    userName: string
    userEmail: string
  }
  providers: ProviderConfig[]
  models: ModelConfig[]
  prompts: {
    commitPrompt: string
  }
}

const props = defineProps<{
  show: boolean
}>()

const emit = defineEmits<{
  close: []
}>()

// Tab state
type SettingsTab = 'general' | 'model' | 'prompt'
const activeTab = ref<SettingsTab>('general')

// Settings data
const settings = reactive<AppSettings>({
  general: { userName: '', userEmail: '' },
  providers: [],
  models: [],
  prompts: { commitPrompt: '' },
})

const loading = ref(false)
const saving = ref(false)
const toast = ref<{ message: string; type: 'error' | 'success' } | null>(null)
let toastTimer: ReturnType<typeof setTimeout> | null = null

function showToast(message: string, type: 'error' | 'success' = 'error') {
  toast.value = { message, type }
  if (toastTimer) clearTimeout(toastTimer)
  toastTimer = setTimeout(() => {
    toast.value = null
  }, 3000)
}

// Selected provider index for model settings
const selectedProviderIdx = ref(0)

const selectedProvider = computed(() => {
  if (settings.providers.length === 0) return null
  const idx = Math.min(selectedProviderIdx.value, settings.providers.length - 1)
  return settings.providers[idx]
})

const modelsForSelectedProvider = computed(() => {
  const prov = selectedProvider.value
  if (!prov) return []
  return settings.models.filter(m => m.providerId === prov.id)
})

// Load settings
async function loadSettings() {
  loading.value = true
  try {
    const data = await invoke<AppSettings>('load_settings')
    Object.assign(settings, data)
    // Also load git config
    try {
      const gitConfig = await invoke<{ userName: string; userEmail: string }>('get_git_config')
      settings.general.userName = gitConfig.userName || settings.general.userName
      settings.general.userEmail = gitConfig.userEmail || settings.general.userEmail
    } catch (_) {
      // git config may not be available if no repo open
    }
  } catch (e: any) {
    showToast(String(e))
  } finally {
    loading.value = false
  }
}

// Save settings
async function saveSettings() {
  saving.value = true
  try {
    await invoke('save_settings', { settings })
    // Save git config too
    try {
      await invoke('set_git_config', {
        userName: settings.general.userName,
        userEmail: settings.general.userEmail,
      })
    } catch (_) {
      // may fail if no repo open
    }
    showToast('设置已保存', 'success')
  } catch (e: any) {
    showToast(String(e))
  } finally {
    saving.value = false
  }
}

// Provider management
let providerIdCounter = 0
function addProvider() {
  providerIdCounter++
  settings.providers.push({
    id: `provider-${Date.now()}-${providerIdCounter}`,
    name: '',
    type: 'openai',
    baseUrl: '',
    apiKey: '',
    isDefault: settings.providers.length === 0,
  })
  selectedProviderIdx.value = settings.providers.length - 1
}

function removeProvider(idx: number) {
  const prov = settings.providers[idx]
  if (!prov) return
  // Remove associated models
  settings.models = settings.models.filter(m => m.providerId !== prov.id)
  settings.providers.splice(idx, 1)
  if (selectedProviderIdx.value >= settings.providers.length) {
    selectedProviderIdx.value = Math.max(0, settings.providers.length - 1)
  }
}

// Model management
let modelIdCounter = 0
function addModel() {
  const prov = selectedProvider.value
  if (!prov) return
  modelIdCounter++
  settings.models.push({
    id: `model-${Date.now()}-${modelIdCounter}`,
    providerId: prov.id,
    name: '',
    isDefault: settings.models.filter(m => m.providerId === prov.id).length === 0,
  })
}

function removeModel(idx: number) {
  settings.models.splice(idx, 1)
}

function setDefaultModel(idx: number) {
  const prov = selectedProvider.value
  if (!prov) return
  const model = settings.models[idx]
  if (!model) return
  // Unset all other models for this provider
  settings.models.forEach(m => {
    if (m.providerId === prov.id) {
      m.isDefault = false
    }
  })
  model.isDefault = true
}

function setDefaultProvider(idx: number) {
  settings.providers.forEach((p, i) => {
    p.isDefault = i === idx
  })
}

// Watch dialog show/hide
watch(() => props.show, (val) => {
  if (val) {
    loadSettings()
  }
})

// Close dialog on Escape
function onKeydown(e: KeyboardEvent) {
  if (e.key === 'Escape' && props.show) {
    emit('close')
  }
}

defineExpose({ showToast })
</script>

<template>
  <Teleport to="body">
    <div
      v-if="show"
      class="fixed inset-0 z-[9999] flex items-center justify-center bg-black/50"
      @click.self="emit('close')"
      @keydown="onKeydown"
      tabindex="-1"
    >
      <div
        class="bg-[--bg-tertiary] border border-[--border-color] rounded-[var(--radius)] shadow-2xl w-[720px] h-[520px] flex flex-col"
        @click.stop
      >
        <!-- Header -->
        <div class="flex items-center justify-between px-4 py-3 border-b border-[--border-color]">
          <div class="flex items-center gap-2 text-sm text-[--text-primary] font-medium">
            <Settings :size="16" />
            <span>设置</span>
          </div>
          <button
            class="p-1 rounded hover:bg-[--bg-secondary] text-[--text-secondary] hover:text-[--text-primary] transition-colors cursor-pointer"
            @click="emit('close')"
          >
            <X :size="16" />
          </button>
        </div>

        <!-- Body: two-column layout -->
        <div class="flex flex-1 overflow-hidden">
          <!-- Left sidebar tabs -->
          <div class="w-[140px] flex-shrink-0 border-r border-[--border-color] p-2 flex flex-col gap-1">
            <button
              class="flex items-center gap-2 px-3 py-2 rounded-[var(--radius)] text-xs transition-colors cursor-pointer"
              :class="activeTab === 'general'
                ? 'bg-[--accent] text-white'
                : 'text-[--text-secondary] hover:text-[--text-primary] hover:bg-[--bg-secondary]'"
              @click="activeTab = 'general'"
            >
              <Globe :size="14" />
              <span>通用设置</span>
            </button>
            <button
              class="flex items-center gap-2 px-3 py-2 rounded-[var(--radius)] text-xs transition-colors cursor-pointer"
              :class="activeTab === 'model'
                ? 'bg-[--accent] text-white'
                : 'text-[--text-secondary] hover:text-[--text-primary] hover:bg-[--bg-secondary]'"
              @click="activeTab = 'model'"
            >
              <Bot :size="14" />
              <span>模型设置</span>
            </button>
            <button
              class="flex items-center gap-2 px-3 py-2 rounded-[var(--radius)] text-xs transition-colors cursor-pointer"
              :class="activeTab === 'prompt'
                ? 'bg-[--accent] text-white'
                : 'text-[--text-secondary] hover:text-[--text-primary] hover:bg-[--bg-secondary]'"
              @click="activeTab = 'prompt'"
            >
              <FileText :size="14" />
              <span>提示词设置</span>
            </button>
          </div>

          <!-- Right content area -->
          <div class="flex-1 overflow-y-auto p-4">
            <!-- Loading -->
            <div v-if="loading" class="flex items-center justify-center h-full text-xs text-[--text-secondary]">
              加载中...
            </div>

            <!-- General Settings -->
            <div v-else-if="activeTab === 'general'" class="space-y-4">
              <div class="text-xs text-[--text-primary] font-medium mb-3">Git 配置</div>
              <div class="space-y-3">
                <div>
                  <label class="block text-[11px] text-[--text-secondary] mb-1">用户名</label>
                  <input
                    v-model="settings.general.userName"
                    placeholder="git config user.name"
                    class="w-full px-3 py-2 rounded-[var(--radius)] bg-[--bg-primary] border border-[--border-color] text-xs text-[--text-primary] outline-none focus:border-[--accent] transition-colors font-mono-ui"
                  />
                </div>
                <div>
                  <label class="block text-[11px] text-[--text-secondary] mb-1">邮箱</label>
                  <input
                    v-model="settings.general.userEmail"
                    placeholder="git config user.email"
                    class="w-full px-3 py-2 rounded-[var(--radius)] bg-[--bg-primary] border border-[--border-color] text-xs text-[--text-primary] outline-none focus:border-[--accent] transition-colors font-mono-ui"
                  />
                </div>
              </div>
              <div class="text-[10px] text-[--text-secondary] leading-relaxed">
                这些设置会写入当前仓库的 git config。如未打开任何仓库，则暂存到本地配置文件中。
              </div>
            </div>

            <!-- Model Settings -->
            <div v-else-if="activeTab === 'model'" class="flex gap-4 h-full">
              <!-- Provider column -->
              <div class="w-[200px] flex-shrink-0 flex flex-col">
                <div class="flex items-center justify-between mb-2">
                  <span class="text-xs text-[--text-primary] font-medium">Providers</span>
                  <button
                    class="p-1 rounded hover:bg-[--bg-primary] text-[--text-secondary] hover:text-[--accent] transition-colors cursor-pointer"
                    title="添加 Provider"
                    @click="addProvider"
                  >
                    <Plus :size="14" />
                  </button>
                </div>
                <div class="flex-1 overflow-y-auto space-y-1">
                  <div
                    v-for="(prov, idx) in settings.providers"
                    :key="prov.id"
                    class="flex items-center gap-2 px-2 py-1.5 rounded-[var(--radius)] text-xs cursor-pointer transition-colors"
                    :class="idx === selectedProviderIdx
                      ? 'bg-[--accent] text-white'
                      : 'text-[--text-primary] hover:bg-[--bg-primary]'"
                    @click="selectedProviderIdx = idx"
                  >
                    <div class="flex-1 min-w-0">
                      <div class="truncate">{{ prov.name || '(未命名)' }}</div>
                      <div class="text-[10px] truncate" :class="idx === selectedProviderIdx ? 'text-white/70' : 'text-[--text-secondary]'">
                        {{ prov.type === 'openai' ? 'OpenAI 兼容' : 'Anthropic 兼容' }}
                      </div>
                    </div>
                    <div class="flex items-center gap-0.5 flex-shrink-0">
                      <Star
                        v-if="prov.isDefault"
                        :size="11"
                        class="text-yellow-400"
                        title="默认 Provider"
                      />
                      <button
                        class="p-0.5 rounded hover:bg-black/20 text-current opacity-60 hover:opacity-100 transition-opacity cursor-pointer"
                        title="删除"
                        @click.stop="removeProvider(idx)"
                      >
                        <Trash2 :size="11" />
                      </button>
                    </div>
                  </div>
                  <div
                    v-if="settings.providers.length === 0"
                    class="text-[11px] text-[--text-secondary] text-center py-4"
                  >
                    暂无 Provider，点击 + 添加
                  </div>
                </div>
              </div>

              <!-- Provider detail + Models column -->
              <div class="flex-1 flex flex-col min-w-0" v-if="selectedProvider">
                <!-- Provider Config -->
                <div class="mb-3 pb-3 border-b border-[--border-color]">
                  <div class="text-xs text-[--text-primary] font-medium mb-2">Provider 配置</div>
                  <div class="space-y-2">
                    <div class="flex gap-2">
                      <div class="flex-1">
                        <label class="block text-[10px] text-[--text-secondary] mb-0.5">名称</label>
                        <input
                          v-model="selectedProvider.name"
                          placeholder="例如: My OpenAI"
                          class="w-full px-2 py-1.5 rounded-[var(--radius)] bg-[--bg-primary] border border-[--border-color] text-xs text-[--text-primary] outline-none focus:border-[--accent] transition-colors"
                        />
                      </div>
                      <div class="w-[130px]">
                        <label class="block text-[10px] text-[--text-secondary] mb-0.5">兼容类型</label>
                        <select
                          v-model="selectedProvider.type"
                          class="w-full px-2 py-1.5 rounded-[var(--radius)] bg-[--bg-primary] border border-[--border-color] text-xs text-[--text-primary] outline-none focus:border-[--accent] transition-colors"
                        >
                          <option value="openai">OpenAI 兼容</option>
                          <option value="anthropic">Anthropic 兼容</option>
                        </select>
                      </div>
                    </div>
                    <div>
                      <label class="block text-[10px] text-[--text-secondary] mb-0.5">Base URL</label>
                      <input
                        v-model="selectedProvider.baseUrl"
                        placeholder="https://api.openai.com/v1"
                        class="w-full px-2 py-1.5 rounded-[var(--radius)] bg-[--bg-primary] border border-[--border-color] text-xs text-[--text-primary] outline-none focus:border-[--accent] transition-colors font-mono-ui"
                      />
                    </div>
                    <div>
                      <label class="block text-[10px] text-[--text-secondary] mb-0.5">API Key</label>
                      <input
                        v-model="selectedProvider.apiKey"
                        type="password"
                        placeholder="sk-..."
                        class="w-full px-2 py-1.5 rounded-[var(--radius)] bg-[--bg-primary] border border-[--border-color] text-xs text-[--text-primary] outline-none focus:border-[--accent] transition-colors font-mono-ui"
                      />
                    </div>
                    <div class="flex items-center gap-2">
                      <label class="flex items-center gap-1.5 cursor-pointer">
                        <input
                          type="checkbox"
                          :checked="selectedProvider.isDefault"
                          @change="setDefaultProvider(selectedProviderIdx)"
                          class="rounded border-[--border-color] bg-[--bg-primary] text-[--accent] focus:ring-[--accent] focus:ring-1"
                        />
                        <span class="text-[11px] text-[--text-secondary]">设为默认 Provider</span>
                      </label>
                    </div>
                  </div>
                </div>

                <!-- Models -->
                <div class="flex-1 min-h-0">
                  <div class="flex items-center justify-between mb-2">
                    <span class="text-xs text-[--text-primary] font-medium">Models</span>
                    <button
                      class="p-1 rounded hover:bg-[--bg-primary] text-[--text-secondary] hover:text-[--accent] transition-colors cursor-pointer"
                      title="添加 Model"
                      @click="addModel"
                    >
                      <Plus :size="14" />
                    </button>
                  </div>
                  <div class="space-y-1">
                    <div
                      v-for="(model, idx) in modelsForSelectedProvider"
                      :key="model.id"
                      class="flex items-center gap-2 px-2 py-1.5 rounded-[var(--radius)] text-xs bg-[--bg-primary] border border-[--border-color]"
                    >
                      <input
                        v-model="model.name"
                        placeholder="模型名称 (如 gpt-4o)"
                        class="flex-1 min-w-0 bg-transparent text-[--text-primary] outline-none font-mono-ui"
                      />
                      <button
                        class="p-0.5 rounded transition-colors cursor-pointer"
                        :class="model.isDefault
                          ? 'text-yellow-400'
                          : 'text-[--text-secondary] hover:text-yellow-400'"
                        :title="model.isDefault ? '默认模型' : '设为默认'"
                        @click="setDefaultModel(idx)"
                      >
                        <Star :size="12" />
                      </button>
                      <button
                        class="p-0.5 rounded text-[--text-secondary] hover:text-red-400 transition-colors cursor-pointer"
                        title="删除"
                        @click="removeModel(idx)"
                      >
                        <Trash2 :size="11" />
                      </button>
                    </div>
                    <div
                      v-if="modelsForSelectedProvider.length === 0"
                      class="text-[11px] text-[--text-secondary] text-center py-4"
                    >
                      暂无 Model，点击 + 添加
                    </div>
                  </div>
                </div>
              </div>

              <!-- No provider selected -->
              <div v-else class="flex-1 flex items-center justify-center text-xs text-[--text-secondary]">
                请选择一个 Provider
              </div>
            </div>

            <!-- Prompt Settings -->
            <div v-else-if="activeTab === 'prompt'" class="flex flex-col h-full">
              <div class="text-xs text-[--text-primary] font-medium mb-3">提交信息提示词</div>
              <div class="text-[11px] text-[--text-secondary] mb-3 leading-relaxed">
                此提示词用于生成 Git Commit Message。AI 将根据你的代码变更和此提示词生成规范的提交信息。
              </div>
              <div class="flex-1 flex flex-col min-h-0">
                <label class="text-[11px] text-[--text-secondary] mb-1">Commit Message 提示词</label>
                <textarea
                  v-model="settings.prompts.commitPrompt"
                  class="flex-1 w-full px-3 py-2 rounded-[var(--radius)] bg-[--bg-primary] border border-[--border-color] text-xs text-[--text-primary] outline-none focus:border-[--accent] transition-colors font-mono-ui resize-none"
                  placeholder="请输入提交提示词..."
                ></textarea>
              </div>
              <div class="mt-2 text-[10px] text-[--text-secondary]">
                提示：修改此提示词将影响 AI 生成的提交信息的格式和风格。
              </div>
            </div>
          </div>
        </div>

        <!-- Footer -->
        <div class="flex items-center justify-end gap-2 px-4 py-3 border-t border-[--border-color]">
          <button
            class="px-3 py-1.5 rounded-[var(--radius)] text-xs text-[--text-secondary] hover:text-[--text-primary] hover:bg-[--bg-secondary] transition-colors cursor-pointer"
            @click="emit('close')"
          >
            取消
          </button>
          <button
            class="px-3 py-1.5 rounded-[var(--radius)] text-xs bg-[--accent] text-white hover:bg-[--accent-hover] transition-colors cursor-pointer disabled:opacity-50"
            :disabled="saving"
            @click="saveSettings"
          >
            {{ saving ? '保存中...' : '保存' }}
          </button>
        </div>

        <!-- Toast -->
        <Transition name="toast">
          <div
            v-if="toast"
            class="absolute bottom-12 right-4 z-[99999] px-2.5 py-2 rounded shadow-lg text-xs max-w-sm break-words"
            :class="toast.type === 'error'
              ? 'bg-red-800 text-red-100 border border-red-700'
              : 'bg-green-800 text-green-100 border border-green-700'"
          >
            {{ toast.message }}
          </div>
        </Transition>
      </div>
    </div>
  </Teleport>
</template>
