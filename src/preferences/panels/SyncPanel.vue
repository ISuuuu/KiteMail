<template>
  <section class="km-tab-panel">
    <div class="km-tab-header-row">
      <h3 class="km-tab-title">同步与通知行为</h3>
      <button type="button" class="km-btn km-btn-secondary" @click="emit('openAccounts')">
        管理邮箱账号…
      </button>
    </div>

    <div class="km-card">
      <div class="km-field-row">
        <label for="km-pref-sync-mins" class="km-label">自动同步周期（分钟，0 为手动）</label>
        <input
          id="km-pref-sync-mins"
          type="number"
          min="0"
          max="1440"
          class="km-input km-num-input"
          :value="state.preferences.value.autoSyncMinutes"
          @change="onNumberInput(state, 'autoSyncMinutes', $event)"
        />
      </div>

      <label class="km-toggle-row">
        <input
          type="checkbox"
          :checked="state.preferences.value.pushEnabled"
          @change="onCheckbox(state, 'pushEnabled', $event)"
        />
        <span>启用实时推送 / IMAP IDLE 监听新邮件</span>
      </label>
      <label class="km-toggle-row">
        <input
          type="checkbox"
          :checked="state.preferences.value.desktopNotifications"
          @change="onCheckbox(state, 'desktopNotifications', $event)"
        />
        <span>收到新邮件时发送系统桌面通知</span>
      </label>
      <label class="km-toggle-row">
        <input
          type="checkbox"
          :checked="state.preferences.value.notificationShowContent"
          @change="onCheckbox(state, 'notificationShowContent', $event)"
        />
        <span>在桌面通知中显示发件人与主题预览</span>
      </label>
      <label class="km-toggle-row">
        <input
          type="checkbox"
          :checked="state.preferences.value.closeToMinimize"
          @change="onCheckbox(state, 'closeToMinimize', $event)"
        />
        <span>点击窗口关闭按钮时最小化而非退出</span>
      </label>
      <label class="km-toggle-row">
        <input
          type="checkbox"
          :checked="state.preferences.value.minimizeToTray"
          @change="onCheckbox(state, 'minimizeToTray', $event)"
        />
        <span>最小化时驻留系统托盘</span>
      </label>
      <label class="km-toggle-row">
        <input
          type="checkbox"
          :checked="state.preferences.value.checkUpdates"
          @change="onCheckbox(state, 'checkUpdates', $event)"
        />
        <span>启动时自动检查版本更新</span>
      </label>
    </div>
  </section>
</template>

<script setup lang="ts">
import { onCheckbox, onNumberInput } from "./fieldBinders";
import type { PrefsPanelState } from "./state";

defineProps<{ state: PrefsPanelState }>();

const emit = defineEmits<{ openAccounts: [] }>();
</script>
