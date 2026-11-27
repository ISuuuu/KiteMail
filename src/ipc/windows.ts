import { invoke, isTauri } from "@tauri-apps/api/core";

/**
 * 窗口管理 IPC（WP-01 新增命令，不属于既有偏好/账号契约）。
 * 浏览器预览（非 Tauri）下全部安全返回 false / 抛错由调用方兜底。
 */

/** 打开（或激活已存在的）独立设置窗口。 */
export async function openSettingsWindow(): Promise<boolean> {
  if (!isTauri()) return false;
  await invoke("open_settings_window");
  return true;
}
