<template>
  <div class="km-win-controls">
    <button
      type="button"
      class="km-win-btn"
      title="最小化"
      aria-label="最小化设置窗口"
      @click="onMinimize"
    >
      <SettingsIcon name="minus" :size="14" />
    </button>
    <button
      type="button"
      class="km-win-btn"
      title="最大化 / 还原"
      aria-label="最大化或还原设置窗口"
      @click="onToggleMaximize"
    >
      <SettingsIcon name="square" :size="12" />
    </button>
    <button
      type="button"
      class="km-win-btn"
      title="关闭设置 (Esc / Ctrl+W)"
      aria-label="关闭设置窗口"
      @click="onClose"
    >
      <SettingsIcon name="close" :size="14" />
    </button>
  </div>
</template>

<script setup lang="ts">
import { getCurrentWindow } from "@tauri-apps/api/window";
import SettingsIcon from "./SettingsIcon.vue";
import { isTauriRuntime } from "./windowEnv";

/** 同排窗体按钮：最小化 / 最大化还原 / 关闭（GPUI render_window_controls）。 */
function onMinimize() {
  if (!isTauriRuntime()) return;
  void getCurrentWindow().minimize();
}

function onToggleMaximize() {
  if (!isTauriRuntime()) return;
  void getCurrentWindow().toggleMaximize();
}

function onClose() {
  if (!isTauriRuntime()) return;
  void getCurrentWindow().close();
}
</script>

<style scoped>
.km-win-controls {
  display: flex;
  align-items: center;
  gap: 2px;
  flex-shrink: 0;
}

.km-win-btn {
  width: 30px;
  height: 30px;
  display: inline-flex;
  align-items: center;
  justify-content: center;
  border: none;
  border-radius: var(--km-radius-sm);
  background: transparent;
  color: var(--km-text-muted);
  cursor: pointer;
  padding: 0;
}

.km-win-btn:hover {
  background: var(--km-hover-bg);
  color: var(--km-text);
}

.km-win-btn:last-child:hover {
  background: color-mix(in srgb, var(--km-danger) 14%, transparent);
  color: var(--km-danger);
}
</style>
