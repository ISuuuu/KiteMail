<template>
  <!-- 独立设置窗：同一前端仅渲染设置窗内容（WP-01），不跑主窗口 bootstrap / 锁屏流程 -->
  <SettingsWindow v-if="isSettings" />
  <div v-else class="km-app" :class="{ locked: state!.locked.value }">
    <MainWindow v-if="!state!.loading.value && !state!.error.value && !state!.locked.value" />
    <div v-else-if="state!.loading.value" class="km-splash">
      <span class="km-splash-spinner" aria-hidden="true" />
      <span class="km-splash-text">正在加载 KiteMail…</span>
      <p v-if="revealError" class="km-error-msg" role="alert">
        窗口显示异常：{{ revealError }}
      </p>
    </div>
    <div v-else-if="state!.error.value" class="km-error-page" role="alert">
      <p class="km-error-title">主窗口初始化失败</p>
      <p class="km-error-msg">{{ state!.error.value }}</p>
      <button type="button" class="km-retry-btn" @click="state!.init()">重试</button>
    </div>
    <LockScreen
      v-else-if="state!.locked.value"
      :hasPasscode="state!.hasPasscode.value"
      :lockedReason="state!.lockedReason.value"
      :submitting="state!.unlockSubmitting.value"
      :error="state!.unlockError.value"
      :retryAfterSecs="state!.unlockRetryAfterSecs.value"
      :mode="state!.mode.value"
      @unlock="state!.unlock"
      @quit="state!.quit"
    />
  </div>
</template>

<script setup lang="ts">
import { nextTick, onMounted, ref, watch } from "vue";
import { revealMainWindow } from "./ipc/windows";
import MainWindow from "./main-window/MainWindow.vue";
import LockScreen from "./main-window/LockScreen.vue";
import SettingsWindow from "./settings/SettingsWindow.vue";
import { provideMainWindow } from "./main-window/useMainWindow";
import { triggerEarlyWindowReveal } from "./main-window/startupHelpers";
import { isSettingsWindow } from "./settings/windowEnv";

// 入口分流一次性判定：设置窗不初始化主窗口状态。
const isSettings = isSettingsWindow();
const state = isSettings ? null : provideMainWindow();
const revealError = ref<string | null>(null);

if (!isSettings && state) {
  let revealed = false;

  const requestReveal = async (source: string) => {
    if (revealed) return;
    const ok = await triggerEarlyWindowReveal({
      source,
      reveal: revealMainWindow,
      onSuccess: (src, stageMs) => {
        revealed = true;
        revealError.value = null;
        console.info(`[startup] revealMainWindow succeeded source=${src} stage_ms=${stageMs.toFixed(1)}`);
      },
      onError: (msg, err) => {
        revealError.value = msg;
        console.error(`[startup] revealMainWindow failed source=${source}:`, err);
      },
    });
    if (!ok) {
      revealed = false;
    }
  };

  // 壳层挂载后立即申请显窗，不等待 main_bootstrap 或隐藏窗口 requestAnimationFrame。
  onMounted(() => {
    const mountMs = typeof performance !== "undefined" ? performance.now() : 0;
    console.info(`[startup] vue_mounted stage_ms=${mountMs.toFixed(1)}`);
    void nextTick(() => {
      void requestReveal("shell-mounted");
    });
  });

  // 若首次显窗失败，待初始化完成后再兜底重试一次，并记录首个可见业务视图。
  watch(
    () => state.loading.value,
    (isLoading) => {
      if (isLoading) return;
      const viewName = state.error.value
        ? "error"
        : state.locked.value
          ? "lock"
          : "main";
      const readyMs = typeof performance !== "undefined" ? performance.now() : 0;
      console.info(`[startup] first_visible_view=${viewName} stage_ms=${readyMs.toFixed(1)}`);
      if (!revealed) {
        void requestReveal(`bootstrap-${viewName}`);
      }
    }
  );
}
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

.km-splash-spinner {
  width: 20px;
  height: 20px;
  border-radius: 999px;
  border: 2.5px solid color-mix(in srgb, var(--km-accent) 25%, transparent);
  border-top-color: var(--km-accent);
  animation: km-splash-spin 0.8s linear infinite;
}

@keyframes km-splash-spin {
  to {
    transform: rotate(360deg);
  }
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
