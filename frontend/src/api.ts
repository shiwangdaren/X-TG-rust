import type { AppSettings } from './types'

const LS_KEY = 'xtg_admin_token'

export function getToken(): string {
  return localStorage.getItem(LS_KEY) ?? ''
}

export function setToken(t: string) {
  if (t) localStorage.setItem(LS_KEY, t)
  else localStorage.removeItem(LS_KEY)
}

function authHeaders(): HeadersInit {
  const t = getToken()
  const h: Record<string, string> = { 'Content-Type': 'application/json' }
  if (t) h['Authorization'] = `Bearer ${t}`
  return h
}

async function parseErr(res: Response): Promise<string> {
  const t = await res.text()
  try {
    const j = JSON.parse(t)
    return j.message ?? j.error ?? t
  } catch {
    return t || res.statusText
  }
}

export async function apiGetSettings(): Promise<AppSettings> {
  const res = await fetch('/api/settings', { headers: authHeaders() })
  if (!res.ok) throw new Error(await parseErr(res))
  return res.json()
}

export async function apiPutSettings(s: AppSettings): Promise<AppSettings> {
  const res = await fetch('/api/settings', {
    method: 'PUT',
    headers: authHeaders(),
    body: JSON.stringify(s),
  })
  if (!res.ok) throw new Error(await parseErr(res))
  return res.json()
}

export async function apiTgConnect(): Promise<void> {
  const res = await fetch('/api/tg/connect', { method: 'POST', headers: authHeaders() })
  if (!res.ok) throw new Error(await parseErr(res))
}

export async function apiTgRequestCode(): Promise<void> {
  const res = await fetch('/api/tg/request-code', { method: 'POST', headers: authHeaders() })
  if (!res.ok) throw new Error(await parseErr(res))
}

export async function apiTgSignIn(code: string): Promise<void> {
  const res = await fetch('/api/tg/sign-in', {
    method: 'POST',
    headers: authHeaders(),
    body: JSON.stringify({ code }),
  })
  if (!res.ok) throw new Error(await parseErr(res))
}

export async function apiTg2fa(password: string): Promise<void> {
  const res = await fetch('/api/tg/2fa', {
    method: 'POST',
    headers: authHeaders(),
    body: JSON.stringify({ password }),
  })
  if (!res.ok) throw new Error(await parseErr(res))
}

export async function apiPollStart(): Promise<void> {
  const res = await fetch('/api/poll/start', { method: 'POST', headers: authHeaders() })
  if (!res.ok) throw new Error(await parseErr(res))
}

export async function apiPollStop(): Promise<void> {
  const res = await fetch('/api/poll/stop', { method: 'POST', headers: authHeaders() })
  if (!res.ok) throw new Error(await parseErr(res))
}

export async function apiStatus(): Promise<import('./types').StatusResponse> {
  const res = await fetch('/api/status', { headers: authHeaders() })
  if (!res.ok) throw new Error(await parseErr(res))
  return res.json()
}

/** 使用 fetch + ReadableStream 读取 SSE（可带 Authorization，兼容 EventSource 无自定义头限制）。 */
export function streamLogs(
  onLine: (line: string) => void,
  signal: AbortSignal,
): Promise<void> {
  return (async () => {
    const res = await fetch('/api/logs/stream', { headers: authHeaders(), signal })
    if (!res.ok) throw new Error(await parseErr(res))
    const reader = res.body!.getReader()
    const dec = new TextDecoder()
    let buf = ''
    while (true) {
      const { done, value } = await reader.read()
      if (done) break
      buf += dec.decode(value, { stream: true })
      const parts = buf.split('\n\n')
      buf = parts.pop() ?? ''
      for (const block of parts) {
        for (const line of block.split('\n')) {
          if (line.startsWith('data:')) {
            onLine(line.slice(5).trimStart())
          }
        }
      }
    }
  })()
}
