<script setup lang="ts">
import { onMounted, onUnmounted, ref } from 'vue'
import type { AppSettings } from './types'
import * as api from './api'

const tokenInput = ref(api.getToken())
const settings = ref<AppSettings | null>(null)
const loginCode = ref('')
const password2fa = ref('')
const logs = ref<string[]>(['加载中…'])
const err = ref('')
const status = ref({ tg_connected: false, poll_running: false, pending_2fa: false })

let logAbort: AbortController | null = null

function pushLog(s: string) {
  logs.value.push(s)
  if (logs.value.length > 500) logs.value.splice(0, 100)
}

function applyToken() {
  api.setToken(tokenInput.value.trim())
  err.value = ''
  load()
}

async function load() {
  try {
    err.value = ''
    settings.value = await api.apiGetSettings()
    const st = await api.apiStatus()
    status.value = st
  } catch (e: unknown) {
    err.value = e instanceof Error ? e.message : String(e)
  }
}

async function save() {
  if (!settings.value) return
  try {
    err.value = ''
    settings.value = await api.apiPutSettings(settings.value)
    pushLog('配置已保存')
    await load()
  } catch (e: unknown) {
    err.value = e instanceof Error ? e.message : String(e)
  }
}

async function connectTg() {
  try {
    err.value = ''
    await save()
    await api.apiTgConnect()
    pushLog('已请求连接 TG')
    await load()
  } catch (e: unknown) {
    err.value = e instanceof Error ? e.message : String(e)
  }
}

async function requestCode() {
  try {
    err.value = ''
    await save()
    await api.apiTgRequestCode()
    pushLog('验证码已发送')
  } catch (e: unknown) {
    err.value = e instanceof Error ? e.message : String(e)
  }
}

async function submitLogin() {
  try {
    err.value = ''
    await api.apiTgSignIn(loginCode.value)
    await load()
    if (status.value.pending_2fa) pushLog('需要二步验证密码')
    else pushLog('Telegram 登录成功')
  } catch (e: unknown) {
    err.value = e instanceof Error ? e.message : String(e)
  }
}

async function submit2fa() {
  try {
    err.value = ''
    await api.apiTg2fa(password2fa.value)
    pushLog('二步验证完成')
    await load()
  } catch (e: unknown) {
    err.value = e instanceof Error ? e.message : String(e)
  }
}

async function startPoll() {
  try {
    err.value = ''
    await save()
    await api.apiPollStart()
    await load()
  } catch (e: unknown) {
    err.value = e instanceof Error ? e.message : String(e)
  }
}

async function stopPoll() {
  try {
    err.value = ''
    await api.apiPollStop()
    await load()
  } catch (e: unknown) {
    err.value = e instanceof Error ? e.message : String(e)
  }
}

function startLogStream() {
  logAbort?.abort()
  logAbort = new AbortController()
  api
    .streamLogs(
      (line) => pushLog(line),
      logAbort.signal,
    )
    .catch((e: unknown) => {
      if ((e as Error).name === 'AbortError') return
      pushLog(`日志流错误: ${e instanceof Error ? e.message : String(e)}`)
    })
}

onMounted(() => {
  load().then(() => startLogStream())
})

onUnmounted(() => {
  logAbort?.abort()
})

const pollPresets = [0.1, 1, 5, 15, 60]
</script>

<template>
  <div class="wrap">
    <header class="hdr">
      <h1>X → Telegram 追踪</h1>
      <p class="sub">管理 API Token（存浏览器 localStorage）</p>
      <div class="row">
        <input v-model="tokenInput" type="password" placeholder="XTG_ADMIN_TOKEN（可选）" class="grow" />
        <button type="button" @click="applyToken">应用 Token</button>
      </div>
      <p v-if="err" class="err">{{ err }}</p>
    </header>

    <section v-if="settings" class="card">
      <h2>Telegram</h2>
      <div class="row">
        <label>API ID</label>
        <input v-model="settings.api_id" />
        <label>API Hash</label>
        <input v-model="settings.api_hash" />
      </div>
      <div class="row">
        <label>手机号</label>
        <input v-model="settings.phone" />
      </div>
      <div class="row">
        <label>session 文件</label>
        <input v-model="settings.tg_session_path" class="grow" />
      </div>
      <div class="row">
        <button type="button" @click="connectTg">连接 TG</button>
        <button type="button" @click="requestCode">请求验证码</button>
      </div>
      <div class="row">
        <label>验证码</label>
        <input v-model="loginCode" />
        <button type="button" @click="submitLogin">提交登录</button>
      </div>
      <div class="row">
        <label>2FA</label>
        <input v-model="password2fa" type="password" />
        <button type="button" @click="submit2fa">提交 2FA</button>
      </div>
      <p class="hint">状态：TG {{ status.tg_connected ? '已连接' : '未连接' }} · 轮询 {{ status.poll_running ? '运行中' : '已暂停' }} · 待 2FA {{ status.pending_2fa ? '是' : '否' }}</p>
    </section>

    <section v-if="settings" class="card">
      <h2>X 与转发</h2>
      <label>TG 目标（每行一个群组/频道；同一帖会发往每一行）</label>
      <textarea v-model="settings.tg_targets" rows="4" class="grow" />
      <label>每行一个 X handle（不含 @）</label>
      <textarea v-model="settings.x_handles" rows="5" class="grow" />
      <div class="row">
        <span>间隔（秒）</span>
        <button v-for="p in pollPresets" :key="p" type="button" class="small" @click="settings.poll_interval_secs = p">{{ p }}s</button>
      </div>
      <div class="row">
        <label>间隔数值</label>
        <input v-model.number="settings.poll_interval_secs" type="number" step="0.1" min="0.1" />
        <label>最大媒体 MB</label>
        <input v-model.number="settings.max_media_mb" type="number" min="1" max="200" />
      </div>
      <div class="row">
        <label><input v-model="settings.use_fake_x" type="checkbox" /> 假 X 数据（测试）</label>
      </div>
      <div class="row">
        <label>X API Base</label>
        <input v-model="settings.x_api_base" class="grow" />
      </div>
      <div class="row">
        <label>X Bearer</label>
        <input v-model="settings.x_bearer_token" type="password" class="grow" />
      </div>
    </section>

    <section v-if="settings" class="card">
      <h2>汉化（Grok）</h2>
      <div class="row">
        <label><input v-model="settings.ai_enabled" type="checkbox" /> 启用 AI 汉化</label>
      </div>
      <p class="hint">使用 xAI Grok；API Base 留空为 https://api.x.ai/v1</p>
      <div class="row">
        <label>API Base</label>
        <input v-model="settings.ai_api_base" class="grow" />
      </div>
      <div class="row">
        <label>API Key</label>
        <input v-model="settings.ai_api_key" type="password" class="grow" />
      </div>
      <div class="row">
        <label>Model</label>
        <input v-model="settings.ai_model" />
      </div>
    </section>

    <section v-if="settings" class="card actions">
      <button type="button" @click="save">保存配置</button>
      <button type="button" @click="startPoll">开始轮询</button>
      <button type="button" @click="stopPoll">停止轮询</button>
    </section>

    <section class="card log">
      <h2>日志</h2>
      <pre>{{ logs.join('\n') }}</pre>
    </section>
  </div>
</template>

<style scoped>
.wrap {
  max-width: 920px;
  margin: 0 auto;
  padding: 1rem;
  font-family: system-ui, sans-serif;
}
.hdr h1 {
  margin: 0 0 0.25rem;
  font-size: 1.35rem;
}
.sub {
  color: #666;
  font-size: 0.9rem;
  margin: 0 0 0.75rem;
}
.row {
  display: flex;
  flex-wrap: wrap;
  gap: 0.5rem;
  align-items: center;
  margin-bottom: 0.5rem;
}
.row label {
  min-width: 6rem;
}
.grow {
  flex: 1;
  min-width: 12rem;
}
input,
textarea,
select,
button {
  font: inherit;
}
button.small {
  padding: 0.15rem 0.4rem;
}
.card {
  border: 1px solid #ddd;
  border-radius: 8px;
  padding: 1rem;
  margin-bottom: 1rem;
}
.card h2 {
  margin: 0 0 0.75rem;
  font-size: 1.1rem;
}
.hint {
  font-size: 0.85rem;
  color: #555;
  margin: 0.5rem 0 0;
}
.err {
  color: #b00020;
  margin: 0.5rem 0 0;
}
.actions {
  display: flex;
  gap: 0.5rem;
  flex-wrap: wrap;
}
.log pre {
  margin: 0;
  white-space: pre-wrap;
  word-break: break-all;
  font-size: 0.8rem;
  max-height: 320px;
  overflow: auto;
  background: #f6f6f6;
  padding: 0.5rem;
  border-radius: 4px;
}
</style>
