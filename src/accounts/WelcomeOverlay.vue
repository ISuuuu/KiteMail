<template>
  <div
    v-if="open"
    class="km-welcome-backdrop"
    role="dialog"
    aria-modal="true"
    aria-labelledby="km-welcome-title"
  >
    <div class="km-welcome-card">
      <!-- 顶部品牌区 -->
      <header class="km-welcome-header">
        <div class="km-welcome-brand">
          <div class="km-welcome-logo" aria-hidden="true">
            <svg viewBox="0 0 28 28" width="24" height="24">
              <path
                fill="currentColor"
                d="M4 6.5A2.5 2.5 0 0 1 6.5 4h15A2.5 2.5 0 0 1 24 6.5v15a2.5 2.5 0 0 1-2.5 2.5h-15A2.5 2.5 0 0 1 4 21.5v-15Zm3 1.75v.44l7 4.66 7-4.66v-.44a.5.5 0 0 0-.5-.5h-13a.5.5 0 0 0-.5.5Zm14 3.05-6.45 4.3a1 1 0 0 1-1.1 0L7 11.3v9.2a.5.5 0 0 0 .5.5h13a.5.5 0 0 0 .5-.5v-9.2Z"
              />
            </svg>
          </div>
          <div>
            <h1 id="km-welcome-title" class="km-welcome-title">欢迎使用 KiteMail</h1>
            <p class="km-welcome-subtitle">快速、干净、本地优先的现代桌面邮件客户端</p>
          </div>
        </div>

        <button
          v-if="step === 'setup'"
          type="button"
          class="km-welcome-back-btn"
          @click="step = 'landing'"
        >
          ← 返回服务商选择
        </button>
      </header>

      <!-- 步骤 1：品牌亮点 + 服务商快捷入口 -->
      <div v-if="step === 'landing'" class="km-welcome-body">
        <section class="km-highlights" aria-label="核心特性">
          <div class="km-highlight-item">
            <span class="km-highlight-icon" aria-hidden="true">📥</span>
            <div>
              <div class="km-highlight-title">多账号统一收件箱</div>
              <div class="km-highlight-desc">集中管理多个邮箱，毫秒级分页检索</div>
            </div>
          </div>
          <div class="km-highlight-item">
            <span class="km-highlight-icon" aria-hidden="true">🔐</span>
            <div>
              <div class="km-highlight-title">OAuth2 与本地加密凭据</div>
              <div class="km-highlight-desc">支持 Microsoft Graph 与应用专用授权码</div>
            </div>
          </div>
          <div class="km-highlight-item">
            <span class="km-highlight-icon" aria-hidden="true">🎨</span>
            <div>
              <div class="km-highlight-title">自适应主题与安全阅读</div>
              <div class="km-highlight-desc">7 套内置主题，隔离渲染远程图片可控</div>
            </div>
          </div>
        </section>

        <section class="km-quick-entry-section">
          <h2 class="km-section-heading">选择您的邮箱服务商开始配置</h2>
          <div class="km-quick-grid">
            <button
              v-for="entry in quickEntries"
              :key="entry.presetId"
              type="button"
              class="km-quick-card"
              :class="{ featured: entry.featured }"
              @click="choosePreset(entry.presetId)"
            >
              <span class="km-quick-badge" :style="{ background: entry.accent }">
                {{ entry.shortLabel }}
              </span>
              <span class="km-quick-meta">
                <span class="km-quick-title">
                  {{ entry.title }}
                  <span v-if="entry.featured" class="km-rec-pill">推荐</span>
                </span>
                <span class="km-quick-desc">{{ entry.desc }}</span>
              </span>
              <span class="km-quick-arrow" aria-hidden="true">→</span>
            </button>
          </div>
        </section>

        <footer v-if="allowSkip" class="km-welcome-footer">
          <button type="button" class="km-link-btn" @click="handleSkip">
            暂不添加，先进入主界面
          </button>
        </footer>
      </div>

      <!-- 步骤 2：无缝衔接账号添加表单 -->
      <form v-else class="km-welcome-form" @submit.prevent="handleSubmit">
        <div v-if="state.activePreset.value" class="km-preset-banner">
          <div class="km-preset-banner-title">
            当前预设：{{ state.activePreset.value.name }}
          </div>
          <div class="km-preset-banner-note">
            {{ state.activePreset.value.note }}
          </div>
        </div>

        <div class="km-form-grid">
          <div class="km-field">
            <label for="km-w-email" class="km-label">邮箱地址 *</label>
            <input
              id="km-w-email"
              ref="emailInputRef"
              :value="state.form.value.email"
              type="email"
              class="km-input"
              placeholder="例如 name@outlook.com 或 123456@qq.com"
              autocomplete="email"
              @input="onEmailInput"
            />
            <span v-if="state.validationErrors.value.email" class="km-field-err">
              {{ state.validationErrors.value.email }}
            </span>
          </div>

          <div class="km-field">
            <label for="km-w-display-name" class="km-label">发件人显示名称 *</label>
            <input
              id="km-w-display-name"
              v-model="state.form.value.displayName"
              type="text"
              class="km-input"
              placeholder="例如 张三"
            />
            <span v-if="state.validationErrors.value.displayName" class="km-field-err">
              {{ state.validationErrors.value.displayName }}
            </span>
          </div>
        </div>

        <!-- Microsoft Graph OAuth2 -->
        <div
          v-if="state.form.value.authBackend === 'microsoftGraphOAuth2'"
          class="km-oauth-panel"
        >
          <p class="km-oauth-hint">
            点击下方按钮将在系统浏览器中安全登录 Microsoft 账号，无需输入密码。
          </p>
          <div class="km-oauth-actions">
            <button
              type="button"
              class="km-btn km-btn-primary"
              :disabled="state.oauthStatus.value === 'pending'"
              @click="state.startMicrosoftOAuthFlow()"
            >
              {{
                state.oauthStatus.value === "pending"
                  ? "正在等待浏览器授权…"
                  : "使用 Microsoft 账号登录授权"
              }}
            </button>
            <button
              type="button"
              class="km-link-btn"
              @click="showAdvanced = !showAdvanced"
            >
              {{ showAdvanced ? "收起高级参数" : "高级参数（自定义 Client ID）" }}
            </button>
          </div>

          <div v-if="showAdvanced" class="km-field km-advanced-box">
            <label for="km-w-client-id" class="km-label">自定义 OAuth Client ID（可选）</label>
            <input
              id="km-w-client-id"
              v-model="state.form.value.oauthClientId"
              type="text"
              class="km-input km-mono"
              placeholder="留空则使用内置默认 Client ID"
            />
          </div>

          <div
            v-if="state.oauthMessage.value"
            class="km-status-box"
            :class="oauthBoxClass"
            role="status"
          >
            {{ state.oauthMessage.value }}
          </div>
        </div>

        <!-- 密码 / 授权码 + IMAP/SMTP 参数 -->
        <template v-else>
          <div class="km-form-grid">
            <div class="km-field">
              <label for="km-w-imap-user" class="km-label">登录用户名 *</label>
              <input
                id="km-w-imap-user"
                :value="state.form.value.imapUsername"
                type="text"
                class="km-input"
                placeholder="通常为完整邮箱地址"
                @input="onImapUserInput"
              />
              <span v-if="state.validationErrors.value.imapUsername" class="km-field-err">
                {{ state.validationErrors.value.imapUsername }}
              </span>
            </div>

            <div class="km-field">
              <label for="km-w-password" class="km-label">密码 / 客户端授权码 *</label>
              <input
                id="km-w-password"
                v-model="state.form.value.imapPassword"
                type="password"
                class="km-input"
                autocomplete="new-password"
                placeholder="请输入密码或 16 位授权码"
              />
              <span v-if="state.validationErrors.value.imapPassword" class="km-field-err">
                {{ state.validationErrors.value.imapPassword }}
              </span>
            </div>
          </div>

          <div class="km-advanced-toggle-row">
            <button
              type="button"
              class="km-link-btn"
              @click="showAdvanced = !showAdvanced"
            >
              {{ showAdvanced ? "▾ 收起 IMAP / SMTP 服务器参数" : "▸ 查看或修改 IMAP / SMTP 服务器参数" }}
            </button>
            <button
              v-if="state.hasLockedPresetFields.value && state.form.value.presetId !== 'manual'"
              type="button"
              class="km-link-btn"
              @click="state.resetToPreset()"
            >
              恢复预设默认
            </button>
          </div>

          <div v-if="showAdvanced" class="km-server-box">
            <div class="km-server-grid">
              <div class="km-field">
                <label for="km-w-imap-host" class="km-label">IMAP 服务器 *</label>
                <input
                  id="km-w-imap-host"
                  :value="state.form.value.imapHost"
                  type="text"
                  class="km-input km-mono"
                  placeholder="imap.example.com"
                  @input="onFieldText('imapHost', $event)"
                />
                <span v-if="state.validationErrors.value.imapHost" class="km-field-err">
                  {{ state.validationErrors.value.imapHost }}
                </span>
              </div>
              <div class="km-field">
                <label for="km-w-imap-port" class="km-label">IMAP 端口 *</label>
                <input
                  id="km-w-imap-port"
                  :value="state.form.value.imapPort"
                  type="number"
                  min="1"
                  max="65535"
                  class="km-input km-mono"
                  @input="onFieldPort('imapPort', $event)"
                />
                <span v-if="state.validationErrors.value.imapPort" class="km-field-err">
                  {{ state.validationErrors.value.imapPort }}
                </span>
              </div>
              <div class="km-field">
                <label for="km-w-imap-sec" class="km-label">IMAP 加密</label>
                <select
                  id="km-w-imap-sec"
                  :value="state.form.value.imapSecurity"
                  class="km-input"
                  @change="onFieldSecurity('imapSecurity', $event)"
                >
                  <option value="tls">SSL/TLS</option>
                  <option value="startTls">STARTTLS</option>
                </select>
              </div>
            </div>

            <div class="km-server-grid">
              <div class="km-field">
                <label for="km-w-smtp-host" class="km-label">SMTP 服务器 *</label>
                <input
                  id="km-w-smtp-host"
                  :value="state.form.value.smtpHost"
                  type="text"
                  class="km-input km-mono"
                  placeholder="smtp.example.com"
                  @input="onFieldText('smtpHost', $event)"
                />
                <span v-if="state.validationErrors.value.smtpHost" class="km-field-err">
                  {{ state.validationErrors.value.smtpHost }}
                </span>
              </div>
              <div class="km-field">
                <label for="km-w-smtp-port" class="km-label">SMTP 端口 *</label>
                <input
                  id="km-w-smtp-port"
                  :value="state.form.value.smtpPort"
                  type="number"
                  min="1"
                  max="65535"
                  class="km-input km-mono"
                  @input="onFieldPort('smtpPort', $event)"
                />
                <span v-if="state.validationErrors.value.smtpPort" class="km-field-err">
                  {{ state.validationErrors.value.smtpPort }}
                </span>
              </div>
              <div class="km-field">
                <label for="km-w-smtp-sec" class="km-label">SMTP 加密</label>
                <select
                  id="km-w-smtp-sec"
                  :value="state.form.value.smtpSecurity"
                  class="km-input"
                  @change="onFieldSecurity('smtpSecurity', $event)"
                >
                  <option value="tls">SSL/TLS</option>
                  <option value="startTls">STARTTLS</option>
                </select>
              </div>
            </div>
          </div>
        </template>

        <div
          v-if="state.statusBanner.value && state.form.value.authBackend !== 'microsoftGraphOAuth2'"
          class="km-status-box"
          :class="`is-${state.statusBanner.value.kind}`"
          role="status"
        >
          {{ state.statusBanner.value.text }}
        </div>

        <footer class="km-form-actions">
          <button
            v-if="allowSkip"
            type="button"
            class="km-btn km-btn-ghost"
            @click="handleSkip"
          >
            跳过
          </button>
          <div class="km-form-actions-right">
            <button
              type="button"
              class="km-btn km-btn-secondary"
              :disabled="state.verifying.value || state.saving.value"
              @click="state.verifyCurrentAccount()"
            >
              {{ state.verifying.value ? "正在测试…" : "测试连接" }}
            </button>
            <button
              type="submit"
              class="km-btn km-btn-primary"
              :disabled="state.saving.value || state.verifying.value"
            >
              {{ state.saving.value ? "正在保存…" : "完成添加" }}
            </button>
          </div>
        </footer>
      </form>
    </div>
  </div>
</template>

<script setup lang="ts">
import { computed, nextTick, onMounted, onUnmounted, ref, watch } from "vue";
import type { AccountConfigDto, SecurityKind } from "./types";
import { useAccounts } from "./useAccounts";

const props = withDefaults(
  defineProps<{
    open?: boolean;
    allowSkip?: boolean;
  }>(),
  {
    open: true,
    allowSkip: false,
  }
);

const emit = defineEmits<{
  completed: [account: AccountConfigDto];
  skip: [];
  overlayChange: [id: string, open: boolean];
}>();

const OVERLAY_ID = "welcome-overlay";

const step = ref<"landing" | "setup">("landing");
const showAdvanced = ref(false);
const emailInputRef = ref<HTMLInputElement | null>(null);

const quickEntries = [
  {
    presetId: "outlook",
    shortLabel: "M",
    title: "Outlook / Microsoft 365",
    desc: "通过 Microsoft OAuth2 安全授权登录",
    accent: "#0078d4",
    featured: true,
  },
  {
    presetId: "gmail",
    shortLabel: "G",
    title: "Gmail",
    desc: "使用 Google 应用专用密码接入 IMAP/SMTP",
    accent: "#d93025",
    featured: false,
  },
  {
    presetId: "qq",
    shortLabel: "Q",
    title: "QQ 邮箱",
    desc: "支持 @qq.com / @foxmail.com 16 位授权码",
    accent: "#12b7f5",
    featured: false,
  },
  {
    presetId: "netease163",
    shortLabel: "163",
    title: "网易 163 邮箱",
    desc: "支持 @163.com 客户端授权密码接入",
    accent: "#d43c33",
    featured: false,
  },
  {
    presetId: "manual",
    shortLabel: "IMAP",
    title: "手动配置 IMAP / SMTP",
    desc: "自定义个人域名、企业邮箱或本地桥接",
    accent: "#5b6472",
    featured: false,
  },
] as const;

const state = useAccounts({
  onOAuthAuthorized(account) {
    if (account) {
      emit("overlayChange", OVERLAY_ID, false);
      emit("completed", account);
    }
  },
});

const oauthBoxClass = computed(() => {
  switch (state.oauthStatus.value) {
    case "authorized":
      return "is-success";
    case "rejected":
    case "error":
      return "is-error";
    default:
      return "is-info";
  }
});

async function choosePreset(presetId: string) {
  await state.init({ initialPresetId: presetId });
  state.startCreate(presetId);
  showAdvanced.value = presetId === "manual";
  step.value = "setup";
  await nextTick();
  emailInputRef.value?.focus();
}

function onEmailInput(ev: Event) {
  const val = (ev.target as HTMLInputElement).value;
  state.updateEmail(val);
}

function onImapUserInput(ev: Event) {
  const val = (ev.target as HTMLInputElement).value;
  state.form.value.imapUsername = val;
  state.markFieldEdited("imapUsername");
}

function onFieldText(field: "imapHost" | "smtpHost", ev: Event) {
  state.form.value[field] = (ev.target as HTMLInputElement).value;
  state.markFieldEdited(field);
}

function onFieldPort(field: "imapPort" | "smtpPort", ev: Event) {
  const raw = (ev.target as HTMLInputElement).value;
  state.form.value[field] = raw === "" ? Number.NaN : Number(raw);
  state.markFieldEdited(field);
}

function onFieldSecurity(field: "imapSecurity" | "smtpSecurity", ev: Event) {
  state.form.value[field] = (ev.target as HTMLSelectElement).value as SecurityKind;
  state.markFieldEdited(field);
}

async function handleSubmit() {
  const saved = await state.saveCurrentAccount();
  if (saved) {
    emit("overlayChange", OVERLAY_ID, false);
    emit("completed", saved);
  } else if (
    state.validationErrors.value.imapHost ||
    state.validationErrors.value.imapPort ||
    state.validationErrors.value.smtpHost ||
    state.validationErrors.value.smtpPort
  ) {
    showAdvanced.value = true;
  }
}

function handleSkip() {
  emit("overlayChange", OVERLAY_ID, false);
  emit("skip");
}

watch(
  () => props.open,
  (isOpen) => {
    emit("overlayChange", OVERLAY_ID, isOpen);
    if (isOpen) {
      step.value = "landing";
      void state.init();
    }
  }
);

onMounted(() => {
  if (props.open) {
    emit("overlayChange", OVERLAY_ID, true);
    void state.init();
  }
});

onUnmounted(() => {
  if (props.open) {
    emit("overlayChange", OVERLAY_ID, false);
  }
  state.dispose();
});
</script>

<style scoped>
.km-welcome-backdrop {
  position: fixed;
  inset: 0;
  z-index: 950;
  display: flex;
  align-items: center;
  justify-content: center;
  padding: 20px;
  background: color-mix(in srgb, var(--km-canvas) 92%, rgba(0, 0, 0, 0.25));
  backdrop-filter: blur(6px);
}

.km-welcome-card {
  width: min(740px, 100%);
  max-height: calc(100vh - 40px);
  overflow-y: auto;
  border-radius: var(--km-radius-lg);
  border: 1px solid var(--km-border);
  background: var(--km-surface-raised);
  box-shadow: var(--km-shadow-popover);
  padding: 26px 28px;
  display: flex;
  flex-direction: column;
  gap: 20px;
}

.km-welcome-header {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 16px;
}

.km-welcome-brand {
  display: flex;
  align-items: center;
  gap: 14px;
}

.km-welcome-logo {
  width: 46px;
  height: 46px;
  border-radius: 12px;
  background: var(--km-accent);
  color: var(--km-accent-fg);
  display: inline-flex;
  align-items: center;
  justify-content: center;
  flex-shrink: 0;
}

.km-welcome-title {
  margin: 0;
  font-size: 20px;
  font-weight: 700;
  color: var(--km-text);
}

.km-welcome-subtitle {
  margin: 2px 0 0;
  font-size: 13px;
  color: var(--km-text-muted);
}

.km-welcome-back-btn {
  border: 1px solid var(--km-border);
  background: var(--km-surface);
  color: var(--km-text);
  border-radius: var(--km-radius-sm);
  padding: 6px 12px;
  cursor: pointer;
  font-size: 12px;
}

.km-welcome-back-btn:hover {
  background: var(--km-hover-bg);
}

.km-welcome-body {
  display: flex;
  flex-direction: column;
  gap: 18px;
}

.km-highlights {
  display: grid;
  grid-template-columns: repeat(3, minmax(0, 1fr));
  gap: 10px;
}

.km-highlight-item {
  display: flex;
  align-items: flex-start;
  gap: 10px;
  padding: 10px 12px;
  border-radius: var(--km-radius-md);
  border: 1px solid var(--km-border);
  background: var(--km-canvas);
}

.km-highlight-icon {
  font-size: 16px;
  line-height: 1.2;
}

.km-highlight-title {
  font-size: 12px;
  font-weight: 600;
  color: var(--km-text);
}

.km-highlight-desc {
  margin-top: 2px;
  font-size: 11px;
  color: var(--km-text-muted);
}

.km-section-heading {
  margin: 0 0 10px;
  font-size: 13px;
  font-weight: 600;
  color: var(--km-text);
}

.km-quick-grid {
  display: grid;
  grid-template-columns: repeat(2, minmax(0, 1fr));
  gap: 10px;
}

.km-quick-card {
  display: flex;
  align-items: center;
  gap: 12px;
  padding: 12px 14px;
  border-radius: var(--km-radius-md);
  border: 1px solid var(--km-border);
  background: var(--km-surface);
  text-align: left;
  cursor: pointer;
  transition: border-color 0.15s ease, background-color 0.15s ease;
}

.km-quick-card.featured {
  grid-column: 1 / -1;
  border-color: color-mix(in srgb, var(--km-accent) 45%, var(--km-border));
  background: var(--km-accent-soft);
}

.km-quick-card:hover {
  border-color: var(--km-accent);
  background: var(--km-hover-bg);
}

.km-quick-card.featured:hover {
  background: var(--km-accent-soft-hover);
}

.km-quick-badge {
  width: 34px;
  height: 34px;
  border-radius: 9px;
  color: #ffffff;
  font-weight: 700;
  font-size: 12px;
  display: inline-flex;
  align-items: center;
  justify-content: center;
  flex-shrink: 0;
}

.km-quick-meta {
  flex: 1;
  min-width: 0;
  display: flex;
  flex-direction: column;
  gap: 2px;
}

.km-quick-title {
  font-size: 13px;
  font-weight: 600;
  color: var(--km-text);
  display: inline-flex;
  align-items: center;
  gap: 6px;
}

.km-rec-pill {
  padding: 1px 6px;
  border-radius: 999px;
  font-size: 10px;
  font-weight: 600;
  background: var(--km-accent);
  color: var(--km-accent-fg);
}

.km-quick-desc {
  font-size: 11px;
  color: var(--km-text-muted);
}

.km-quick-arrow {
  color: var(--km-text-muted);
  font-size: 14px;
}

.km-welcome-footer {
  display: flex;
  justify-content: flex-end;
}

.km-welcome-form {
  display: flex;
  flex-direction: column;
  gap: 14px;
}

.km-preset-banner {
  padding: 10px 12px;
  border-radius: var(--km-radius-md);
  background: var(--km-accent-soft);
  border: 1px solid color-mix(in srgb, var(--km-accent) 30%, var(--km-border));
}

.km-preset-banner-title {
  font-size: 12px;
  font-weight: 600;
  color: var(--km-text);
}

.km-preset-banner-note {
  margin-top: 2px;
  font-size: 12px;
  color: var(--km-text-muted);
}

.km-form-grid {
  display: grid;
  grid-template-columns: repeat(2, minmax(0, 1fr));
  gap: 12px;
}

.km-server-box {
  display: flex;
  flex-direction: column;
  gap: 10px;
  padding: 12px;
  border-radius: var(--km-radius-md);
  border: 1px solid var(--km-border);
  background: var(--km-canvas);
}

.km-server-grid {
  display: grid;
  grid-template-columns: 2fr 1fr 1.1fr;
  gap: 10px;
}

.km-field {
  display: flex;
  flex-direction: column;
  gap: 4px;
  min-width: 0;
}

.km-label {
  font-size: 12px;
  font-weight: 500;
  color: var(--km-text);
}

.km-input {
  width: 100%;
  height: 34px;
  padding: 0 10px;
  border: 1px solid var(--km-border);
  border-radius: var(--km-radius-sm);
  background: var(--km-surface);
  color: var(--km-text);
  user-select: text;
}

.km-input:focus {
  border-color: var(--km-accent);
  outline: none;
}

.km-mono {
  font-family: var(--km-font-mono);
  font-size: 12px;
}

.km-field-err {
  font-size: 11px;
  color: var(--km-danger);
}

.km-oauth-panel {
  display: flex;
  flex-direction: column;
  gap: 10px;
  padding: 14px;
  border-radius: var(--km-radius-md);
  border: 1px solid var(--km-border);
  background: var(--km-canvas);
}

.km-oauth-hint {
  margin: 0;
  font-size: 12px;
  color: var(--km-text-muted);
}

.km-oauth-actions {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 12px;
  flex-wrap: wrap;
}

.km-advanced-box {
  margin-top: 4px;
}

.km-advanced-toggle-row {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 12px;
}

.km-status-box {
  padding: 9px 12px;
  border-radius: var(--km-radius-sm);
  font-size: 12px;
  border: 1px solid var(--km-border);
  background: var(--km-canvas);
}

.km-status-box.is-success {
  border-color: color-mix(in srgb, #26a269 45%, var(--km-border));
  background: color-mix(in srgb, #26a269 12%, transparent);
}

.km-status-box.is-error {
  border-color: color-mix(in srgb, var(--km-danger) 50%, var(--km-border));
  background: var(--km-danger-soft);
  color: var(--km-danger);
}

.km-status-box.is-info {
  border-color: color-mix(in srgb, var(--km-accent) 45%, var(--km-border));
  background: var(--km-accent-soft);
}

.km-form-actions {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 12px;
  margin-top: 4px;
}

.km-form-actions-right {
  margin-left: auto;
  display: flex;
  align-items: center;
  gap: 8px;
}

.km-btn {
  height: 34px;
  padding: 0 14px;
  border-radius: var(--km-radius-sm);
  border: 1px solid var(--km-border);
  font-weight: 600;
  cursor: pointer;
}

.km-btn:disabled {
  opacity: 0.55;
  cursor: not-allowed;
}

.km-btn-primary {
  border-color: var(--km-accent);
  background: var(--km-accent);
  color: var(--km-accent-fg);
}

.km-btn-secondary {
  background: var(--km-surface);
  color: var(--km-text);
}

.km-btn-secondary:hover:not(:disabled) {
  background: var(--km-hover-bg);
}

.km-btn-ghost {
  border-color: transparent;
  background: transparent;
  color: var(--km-text-muted);
}

.km-link-btn {
  border: none;
  background: transparent;
  color: var(--km-accent);
  font-size: 12px;
  cursor: pointer;
  padding: 2px 4px;
}

.km-link-btn:hover {
  text-decoration: underline;
}

@media (max-width: 680px) {
  .km-highlights,
  .km-quick-grid,
  .km-form-grid,
  .km-server-grid {
    grid-template-columns: 1fr;
  }
}
</style>
