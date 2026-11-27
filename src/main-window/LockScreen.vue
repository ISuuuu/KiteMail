<template>
  <main class="km-lock-screen" aria-label="应用锁屏界面">
    <section class="km-lock-card">
      <div class="km-lock-icon" aria-hidden="true">🔒</div>
      <h1 class="km-lock-title">KiteMail 已锁定</h1>
      <p class="km-lock-desc">{{ reasonHint }}</p>

      <form class="km-lock-form" @submit.prevent="handleSubmit">
        <label class="km-lock-label" for="km-unlock-passcode">锁屏密码</label>
        <input
          id="km-unlock-passcode"
          ref="inputRef"
          v-model="passcode"
          type="password"
          class="km-lock-input"
          autocomplete="current-password"
          placeholder="请输入锁屏密码"
          :disabled="submitting || retryAfterSecs > 0"
          @keydown.enter.prevent="handleSubmit"
        />

        <p v-if="retryAfterSecs > 0" class="km-lock-cooldown" role="status">
          尝试次数过多，请 {{ retryAfterSecs }} 秒后再试
        </p>
        <p v-else-if="error" class="km-lock-error" role="alert">
          {{ error }}
        </p>

        <button
          type="submit"
          class="km-unlock-btn"
          :disabled="submitting || retryAfterSecs > 0 || passcode.length === 0"
        >
          {{
            submitting
              ? "正在解锁…"
              : retryAfterSecs > 0
                ? `请等待 ${retryAfterSecs}s`
                : "解锁"
          }}
        </button>
      </form>

      <p v-if="mode === 'demo'" class="km-demo-hint">演示模式默认密码：1234</p>

      <button type="button" class="km-quit-btn" @click="emit('quit')">
        退出应用
      </button>
    </section>
  </main>
</template>

<script setup lang="ts">
import { computed, onMounted, onUnmounted, ref } from "vue";

const props = defineProps<{
  hasPasscode?: boolean;
  lockedReason?: string | null;
  submitting: boolean;
  error: string | null;
  retryAfterSecs: number;
  mode?: "demo" | "live";
}>();

const emit = defineEmits<{
  unlock: [passcode: string];
  quit: [];
}>();

const passcode = ref("");
const inputRef = ref<HTMLInputElement | null>(null);

const reasonHint = computed(() => {
  switch (props.lockedReason) {
    case "idle":
      return "由于长时间未操作，应用已自动锁定";
    case "background":
      return "应用切入后台后已自动锁定";
    case "manual":
      return "已手动锁定，请输入密码继续查看邮件";
    default:
      return "解锁后可继续查看邮件内容";
  }
});

function handleSubmit() {
  if (props.submitting || props.retryAfterSecs > 0 || !passcode.value) return;
  const value = passcode.value;
  passcode.value = "";
  emit("unlock", value);
}

onMounted(() => {
  inputRef.value?.focus();
});

onUnmounted(() => {
  passcode.value = "";
});
</script>

<style scoped>
.km-lock-screen {
  flex: 1 1 auto;
  display: flex;
  align-items: center;
  justify-content: center;
  padding: 24px;
  background: var(--km-canvas);
}

.km-lock-card {
  width: 100%;
  max-width: 360px;
  padding: 28px 24px 22px;
  border-radius: var(--km-radius-lg);
  border: 1px solid var(--km-border);
  background: var(--km-surface-raised);
  box-shadow: var(--km-shadow-popover);
  display: flex;
  flex-direction: column;
  align-items: center;
  gap: 10px;
}

.km-lock-icon {
  width: 48px;
  height: 48px;
  border-radius: 999px;
  background: var(--km-accent-soft);
  color: var(--km-accent);
  display: inline-flex;
  align-items: center;
  justify-content: center;
  font-size: 22px;
}

.km-lock-title {
  margin: 4px 0 0;
  font-size: 18px;
  font-weight: 700;
  color: var(--km-text);
}

.km-lock-desc {
  margin: 0;
  font-size: 13px;
  color: var(--km-text-muted);
  text-align: center;
}

.km-lock-form {
  width: 100%;
  margin-top: 8px;
  display: flex;
  flex-direction: column;
  gap: 8px;
}

.km-lock-label {
  position: absolute;
  width: 1px;
  height: 1px;
  overflow: hidden;
  clip: rect(0 0 0 0);
}

.km-lock-input {
  width: 100%;
  height: 36px;
  padding: 0 12px;
  border: 1px solid var(--km-border);
  border-radius: var(--km-radius-md);
  background: var(--km-canvas);
  color: var(--km-text);
  font-size: 14px;
}

.km-lock-input:focus {
  border-color: var(--km-accent);
  outline: none;
}

.km-lock-error,
.km-lock-cooldown {
  margin: 0;
  font-size: 12px;
  color: var(--km-danger);
  text-align: center;
}

.km-unlock-btn {
  height: 36px;
  border: none;
  border-radius: var(--km-radius-md);
  background: var(--km-accent);
  color: var(--km-accent-fg);
  font-weight: 600;
  cursor: pointer;
}

.km-unlock-btn:disabled {
  opacity: 0.55;
  cursor: not-allowed;
}

.km-demo-hint {
  margin: 2px 0 0;
  font-size: 11px;
  color: var(--km-text-muted);
}

.km-quit-btn {
  margin-top: 2px;
  border: none;
  background: transparent;
  color: var(--km-text-muted);
  font-size: 12px;
  cursor: pointer;
  padding: 4px 8px;
}

.km-quit-btn:hover {
  color: var(--km-text);
}
</style>
