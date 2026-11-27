<template>
  <div class="km-app" :class="{ locked: state.locked.value }">
    <MainWindow v-if="!state.loading.value && !state.error.value && !state.locked.value" />
    <div v-else-if="state.loading.value" class="km-splash">
      <span class="km-splash-text">正在加载 KiteMail…</span>
    </div>
    <div v-else-if="state.error.value" class="km-error-page" role="alert">
      <p class="km-error-title">主窗口初始化失败</p>
      <p class="km-error-msg">{{ state.error.value }}</p>
      <button type="button" class="km-retry-btn" @click="state.init()">重试</button>
    </div>
    <LockScreen
      v-else-if="state.locked.value"
      :hasPasscode="state.hasPasscode.value"
      :lockedReason="state.lockedReason.value"
      :submitting="state.unlockSubmitting.value"
      :error="state.unlockError.value"
      :retryAfterSecs="state.unlockRetryAfterSecs.value"
      :mode="state.mode.value"
      @unlock="state.unlock"
      @quit="state.quit"
    />
  </div>
</template>

<script setup lang="ts">
import MainWindow from "./main-window/MainWindow.vue";
import LockScreen from "./main-window/LockScreen.vue";
import { provideMainWindow } from "./main-window/useMainWindow";

const state = provideMainWindow();
</script>

<style scoped>
.km-app {
  width: 100%;
  height: 100%;
  display: flex;
  flex-direction: column;
  background: var(--km-canvas);
  color: var(--km-text);
}

.km-splash,
.km-error-page,
.km-lock-screen {
  flex: 1;
  display: flex;
  flex-direction: column;
  align-items: center;
  justify-content: center;
  gap: 8px;
  color: var(--km-text-muted);
  padding: 24px;
}

.km-splash-text {
  font-size: 14px;
}

.km-error-title,
.km-lock-title {
  font-size: 16px;
  font-weight: 600;
  color: var(--km-text);
  margin: 0;
}

.km-error-title {
  color: var(--km-danger);
}

.km-error-msg,
.km-lock-desc {
  font-size: 13px;
  color: var(--km-text-muted);
  margin: 0;
  max-width: 480px;
  text-align: center;
  word-break: break-word;
}

.km-retry-btn {
  margin-top: 8px;
  padding: 6px 16px;
  border: 1px solid var(--km-border);
  border-radius: var(--km-radius-md);
  background: var(--km-accent);
  color: var(--km-accent-fg);
  cursor: pointer;
}
</style>
