<template>
  <section class="km-tab-panel">
    <h3 class="km-tab-title">隐私保护、远图白名单与安全锁</h3>

    <div class="km-card">
      <div class="km-card-title">远程图片与链接跟踪防护</div>
      <label class="km-toggle-row">
        <input
          type="checkbox"
          :checked="state.preferences.value.blockRemoteImages"
          @change="onCheckbox(state, 'blockRemoteImages', $event)"
        />
        <span>默认拦截 HTML 邮件中的外部远程图片</span>
      </label>
      <label class="km-toggle-row">
        <input
          type="checkbox"
          :checked="state.preferences.value.showRemoteBanner"
          @change="onCheckbox(state, 'showRemoteBanner', $event)"
        />
        <span>拦截远程图片时在阅读器顶部显示放行提示条</span>
      </label>
      <label class="km-toggle-row">
        <input
          type="checkbox"
          :checked="state.preferences.value.cleanLinks"
          @change="onCheckbox(state, 'cleanLinks', $event)"
        />
        <span>自动清理常见邮件跟踪链接参数 (UTM / fbclid)</span>
      </label>
    </div>

    <!-- 远图信任发件人白名单 -->
    <div class="km-card">
      <div class="km-card-header-row">
        <div class="km-card-title">
          远图白名单发件人 ({{ state.allowedRemoteSenders.value.length }})
        </div>
        <button
          v-if="state.allowedRemoteSenders.value.length > 0"
          type="button"
          class="km-btn km-btn-ghost km-danger-text"
          :disabled="state.sendersBusy.value"
          @click="state.clearTrustedSenders()"
        >
          清空全部
        </button>
      </div>

      <div class="km-inline-add">
        <input
          v-model="state.newTrustedSenderInput.value"
          type="email"
          class="km-input"
          placeholder="输入信任的发件人邮箱，如 newsletter@example.com"
          @keydown.enter.prevent="state.addTrustedSender()"
        />
        <button
          type="button"
          class="km-btn km-btn-secondary"
          :disabled="state.sendersBusy.value"
          @click="state.addTrustedSender()"
        >
          添加
        </button>
      </div>

      <div v-if="state.allowedRemoteSenders.value.length === 0" class="km-muted-hint">
        暂无白名单发件人，所有外部远程图片默认受拦截保护。
      </div>
      <ul v-else class="km-sender-list">
        <li
          v-for="sender in state.allowedRemoteSenders.value"
          :key="sender"
          class="km-sender-item"
        >
          <span class="km-mono">{{ sender }}</span>
          <button
            type="button"
            class="km-btn km-btn-ghost km-danger-text"
            :disabled="state.sendersBusy.value"
            @click="state.removeTrustedSender(sender)"
          >
            移除
          </button>
        </li>
      </ul>
    </div>

    <!-- 应用安全锁口令 -->
    <div class="km-card">
      <div class="km-card-header-row">
        <div class="km-card-title">应用安全锁与口令</div>
        <span class="km-status-pill" :class="{ active: state.hasPasscode.value }">
          {{ state.hasPasscode.value ? "已设置口令" : "未设置口令" }}
        </span>
      </div>

      <div class="km-grid-2">
        <div v-if="state.hasPasscode.value" class="km-field">
          <label for="km-lock-current" class="km-label">当前口令 *</label>
          <input
            id="km-lock-current"
            v-model="state.currentPasscodeInput.value"
            type="password"
            class="km-input"
            autocomplete="current-password"
            placeholder="验证当前口令"
          />
        </div>
        <div class="km-field">
          <label for="km-lock-new" class="km-label">
            {{ state.hasPasscode.value ? "新口令（至少 4 位）" : "设置口令（至少 4 位）" }}
          </label>
          <input
            id="km-lock-new"
            v-model="state.newPasscodeInput.value"
            type="password"
            class="km-input"
            autocomplete="new-password"
            placeholder="输入新安全锁口令"
          />
        </div>
        <div class="km-field">
          <label for="km-lock-confirm" class="km-label">确认新口令</label>
          <input
            id="km-lock-confirm"
            v-model="state.confirmPasscodeInput.value"
            type="password"
            class="km-input"
            autocomplete="new-password"
            placeholder="再次输入新口令"
          />
        </div>
      </div>

      <label class="km-toggle-row">
        <input
          type="checkbox"
          :checked="state.preferences.value.appLockOnStartup"
          :disabled="!state.hasPasscode.value"
          @change="onCheckbox(state, 'appLockOnStartup', $event)"
        />
        <span>启动应用时自动进入锁屏状态</span>
      </label>

      <div class="km-field-row">
        <label for="km-pref-idle-mins" class="km-label">闲置自动锁定时间（分钟）</label>
        <input
          id="km-pref-idle-mins"
          type="number"
          min="1"
          max="240"
          class="km-input km-num-input"
          :value="state.preferences.value.appLockIdleMinutes"
          @change="onNumberInput(state, 'appLockIdleMinutes', $event)"
        />
      </div>

      <div v-if="state.passcodeError.value" class="km-field-err" role="alert">
        {{ state.passcodeError.value }}
      </div>

      <div class="km-inline-actions">
        <button
          type="button"
          class="km-btn km-btn-primary"
          :disabled="state.passcodeBusy.value"
          @click="state.submitPasscode()"
        >
          {{ state.hasPasscode.value ? "更新安全锁口令" : "设置并启用口令" }}
        </button>
        <button
          v-if="state.hasPasscode.value"
          type="button"
          class="km-btn km-btn-danger-outline"
          :disabled="state.passcodeBusy.value"
          @click="state.clearPasscode()"
        >
          清除口令
        </button>
      </div>
    </div>
  </section>
</template>

<script setup lang="ts">
import { onCheckbox, onNumberInput } from "./fieldBinders";
import type { PrefsPanelState } from "./state";

defineProps<{ state: PrefsPanelState }>();
</script>
