export interface AppSettings {
  api_id: string
  api_hash: string
  phone: string
  tg_session_path: string
  /** 每行一个群组/频道 */
  tg_targets: string
  x_handles: string
  poll_interval_secs: number
  max_media_mb: number
  use_fake_x: boolean
  x_bearer_token: string
  x_api_base: string
  ai_enabled: boolean
  ai_api_base: string
  ai_api_key: string
  ai_model: string
}

export interface StatusResponse {
  tg_connected: boolean
  poll_running: boolean
  pending_2fa: boolean
}
