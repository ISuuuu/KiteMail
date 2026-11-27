<template>
  <div
    v-if="open"
    class="km-accounts-backdrop"
    role="dialog"
    aria-modal="true"
    aria-labelledby="km-accounts-modal-title"
    @click.self="closeModal"
  >
    <section class="km-accounts-modal">
      <!-- 顶部栏 -->
      <header class="km-modal-header">
        <div class="km-modal-title-group">
          <h2 id="km-accounts-modal-title" class="km-modal-title">邮箱账号管理</h2>
          <span class="km-modal-count">{{ state.accounts.value.length }} 个已配置账号</span>
        </div>

        <div class="km-modal-header-actions">
          <button
            type="button"
            class="km-btn km-btn-secondary"
            @click="state.startCreate()"
          >
            + 添加账号
          </button>
          <button
            type="button"
            class="km-close-btn"
            title="关闭 (Esc)"
            aria-label="关闭账号管理窗口"
            @click="closeModal"
          >
            ✕
          </button>
        </div>
      </header>

      <!-- 主体双栏：左侧账号列表 + 右侧表单 -->
      <div class="km-modal-body">
        <!-- 左侧：账号列表 -->
        <aside class="km-account-sidebar" aria-label="已配置账号列表">
          <div v-if="state.loading.value" class="km-sidebar-empty">
            正在加载账号列表…
          </div>
          <div v-else-if="state.loadError.value" class="km-sidebar-error" role="alert">
            <p>{{ state.loadError.value }}</p>
            <button type="button" class="km-btn km-btn-secondary" @click="state.loadAccounts()">
              重试
            </button>
          </div>
          <div v-else-if="state.accounts.value.length === 0" class="km-sidebar-empty">
            <p>暂无已配置邮箱账号</p>
            <button type="button" class="km-btn km-btn-primary" @click="state.startCreate()">
              立即添加
            </button>
          </div>
          <ul v-else class="km-account-list">
            <li
              v-for="acc in state.accounts.value"
              :key="acc.id"
              class="km-account-item"
              :class="{
                active: !state.isCreating.value && state.selectedAccountId.value === acc.id,
                disabled: !acc.enabled,
              }"
            >
              <button
                type="button"
                class="km-account-select-btn"
                @click="state.selectAccount(acc.id)"
              >
                <span class="km-acc-avatar" aria-hidden="true">
                  {{ avatarInitial(acc.label || acc.displayName || acc.email) }}
                </span>
                <span class="km-acc-info">
                  <span class="km-acc-name-row">
                    <span class="km-acc-name">{{ acc.label || acc.displayName }}</span>
                    <span class="km-acc-tag">
                      {{ acc.authBackend === "microsoftGraphOAuth2" ? "OAuth2" : "IMAP" }}
                    </span>
                  </span>
                  <span class="km-acc-email">{{ acc.email }}</span>
                </span>
              </button>

              <button
                type="button"
                role="switch"
                class="km-switch"
                :class="{ checked: acc.enabled }"
                :aria-checked="acc.enabled"
                :aria-label="acc.enabled ? `停用 ${acc.email}` : `启用 ${acc.email}`"
                :title="acc.enabled ? '已启用同步（点击停用）' : '已停用同步（点击启用）'"
                :disabled="state.togglingEnabledId.value === acc.id"
                @click.stop="state.toggleAccountEnabled(acc.id)"
              >
                <span class="km-switch-thumb" />
              </button>
            </li>
          </ul>
        </aside>

        <!-- 右侧：新增 / 编辑表单 -->
        <main class="km-account-editor">
          <form class="km-editor-form" @submit.prevent="handleSave">
            <!-- 表单标题与服务商预设选择 -->
            <div class="km-editor-header">
              <div>
                <h3 class="km-editor-title">
                  {{
                    state.isCreating.value
                      ? "添加邮箱账号"
                      : `编辑账号：${state.selectedAccount.value?.email ?? ""}`
                  }}
                </h3>
                <p class="km-editor-subtitle">
                  输入邮箱地址时将自动识别主流邮箱服务商并填充服务器参数
                </p>
              </div>

              <div class="km-preset-picker">
                <label for="km-acc-preset" class="km-sr-only">邮箱服务商预设</label>
                <select
                  id="km-acc-preset"
                  :value="state.form.value.presetId"
                  class="km-input km-preset-select"
                  @change="onPresetChange"
                >
                  <option
                    v-for="preset in state.presets.value"
                    :key="preset.id"
                    :value="preset.id"
                  >
                    {{ preset.name }}
                  </option>
                </select>
                <button
                  v-if="
                    state.hasLockedPresetFields.value &&
                    state.activePreset.value &&
                    state.activePreset.value.id !== 'manual'
                  "
                  type="button"
                  class="km-btn km-btn-ghost km-reset-btn"
                  title="清空手动修改，恢复当前预设默认参数"
                  @click="state.resetToPreset()"
                >
                  重置预设
                </button>
              </div>
            </div>

            <!-- 服务商说明条 -->
            <div v-if="state.activePreset.value?.note" class="km-note-banner">
              {{ state.activePreset.value.note }}
            </div>

            <!-- 基础信息 -->
            <section class="km-form-section">
              <h4 class="km-section-title">基础信息</h4>
              <div class="km-grid-2">
                <div class="km-field">
                  <label for="km-acc-email" class="km-label">邮箱地址 *</label>
                  <input
                    id="km-acc-email"
                    ref="emailInputRef"
                    :value="state.form.value.email"
                    type="email"
                    class="km-input"
                    placeholder="name@example.com"
                    autocomplete="email"
                    @input="onEmailInput"
                  />
                  <span v-if="state.validationErrors.value.email" class="km-field-err">
                    {{ state.validationErrors.value.email }}
                  </span>
                </div>

                <div class="km-field">
                  <label for="km-acc-display" class="km-label">发件人显示名称 *</label>
                  <input
                    id="km-acc-display"
                    v-model="state.form.value.displayName"
                    type="text"
                    class="km-input"
                    placeholder="例如 张三"
                  />
                  <span v-if="state.validationErrors.value.displayName" class="km-field-err">
                    {{ state.validationErrors.value.displayName }}
                  </span>
                </div>

                <div class="km-field">
                  <label for="km-acc-label" class="km-label">账号备注名称（可选）</label>
                  <input
                    id="km-acc-label"
                    v-model="state.form.value.label"
                    type="text"
                    class="km-input"
                    placeholder="例如 工作 / 个人"
                  />
                </div>

                <div class="km-field km-toggles-col">
                  <label class="km-checkbox-row">
                    <input v-model="state.form.value.enabled" type="checkbox" />
                    <span>启用此账号收发同步</span>
                  </label>
                  <label class="km-checkbox-row">
                    <input v-model="state.form.value.inUnified" type="checkbox" />
                    <span>在统一收件箱与聚合视图中展示</span>
                  </label>
                </div>
              </div>
            </section>

            <!-- 认证方式 -->
            <section class="km-form-section">
              <div class="km-section-header-row">
                <h4 class="km-section-title">认证与连接方式</h4>
                <div class="km-auth-tabs" role="tablist" aria-label="认证方式">
                  <button
                    type="button"
                    role="tab"
                    class="km-auth-tab"
                    :aria-selected="state.form.value.authBackend === 'password'"
                    :class="{ active: state.form.value.authBackend === 'password' }"
                    @click="setAuthBackend('password')"
                  >
                    密码 / 授权码 (IMAP)
                  </button>
                  <button
                    type="button"
                    role="tab"
                    class="km-auth-tab"
                    :aria-selected="state.form.value.authBackend === 'microsoftGraphOAuth2'"
                    :class="{ active: state.form.value.authBackend === 'microsoftGraphOAuth2' }"
                    @click="setAuthBackend('microsoftGraphOAuth2')"
                  >
                    Microsoft Graph OAuth2
                  </button>
                </div>
              </div>

              <!-- OAuth2 面板 -->
              <div
                v-if="state.form.value.authBackend === 'microsoftGraphOAuth2'"
                class="km-oauth-box"
              >
                <div class="km-oauth-top">
                  <div>
                    <div class="km-oauth-title">Microsoft 365 / Outlook OAuth2 授权</div>
                    <div class="km-oauth-desc">
                      通过系统浏览器完成登录授权，凭证安全保存于 Windows 凭据管理器。
                    </div>
                  </div>
                  <button
                    type="button"
                    class="km-btn km-btn-primary"
                    :disabled="state.oauthStatus.value === 'pending'"
                    @click="state.startMicrosoftOAuthFlow()"
                  >
                    {{
                      state.oauthStatus.value === "pending"
                        ? "等待授权回调…"
                        : state.isCreating.value
                          ? "Microsoft 登录授权"
                          : "重新授权 Microsoft 账号"
                    }}
                  </button>
                </div>

                <div class="km-field">
                  <label for="km-acc-client-id" class="km-label">
                    自定义 OAuth Client ID（可选，留空使用内置）
                  </label>
                  <input
                    id="km-acc-client-id"
                    v-model="state.form.value.oauthClientId"
                    type="text"
                    class="km-input km-mono"
                    placeholder="xxxxxxxx-xxxx-xxxx-xxxx-xxxxxxxxxxxx"
                  />
                </div>

                <div
                  v-if="state.oauthMessage.value"
                  class="km-banner"
                  :class="oauthBannerClass"
                  role="status"
                >
                  {{ state.oauthMessage.value }}
                </div>
              </div>

              <!-- IMAP / SMTP 密码模式 -->
              <template v-else>
                <div class="km-grid-2">
                  <div class="km-field">
                    <label for="km-acc-imap-user" class="km-label">IMAP 用户名 *</label>
                    <input
                      id="km-acc-imap-user"
                      :value="state.form.value.imapUsername"
                      type="text"
                      class="km-input"
                      placeholder="通常为完整邮箱地址"
                      @input="onImapUsernameInput"
                    />
                    <span v-if="state.validationErrors.value.imapUsername" class="km-field-err">
                      {{ state.validationErrors.value.imapUsername }}
                    </span>
                  </div>

                  <div class="km-field">
                    <label for="km-acc-imap-pwd" class="km-label">
                      {{
                        state.form.value.id && state.form.value.hasPassword
                          ? "密码 / 授权码（留空保留原密码）"
                          : "密码 / 授权码 *"
                      }}
                    </label>
                    <input
                      id="km-acc-imap-pwd"
                      v-model="state.form.value.imapPassword"
                      type="password"
                      class="km-input"
                      autocomplete="new-password"
                      :placeholder="
                        state.form.value.id && state.form.value.hasPassword
                          ? '已保存密码，留空则保持不变'
                          : '请输入邮箱密码或授权码'
                      "
                    />
                    <span v-if="state.validationErrors.value.imapPassword" class="km-field-err">
                      {{ state.validationErrors.value.imapPassword }}
                    </span>
                  </div>
                </div>

                <div class="km-grid-server">
                  <div class="km-field">
                    <label for="km-acc-imap-host" class="km-label">IMAP 服务器 *</label>
                    <input
                      id="km-acc-imap-host"
                      :value="state.form.value.imapHost"
                      type="text"
                      class="km-input km-mono"
                      placeholder="imap.example.com"
                      @input="onServerText('imapHost', $event)"
                    />
                    <span v-if="state.validationErrors.value.imapHost" class="km-field-err">
                      {{ state.validationErrors.value.imapHost }}
                    </span>
                  </div>
                  <div class="km-field">
                    <label for="km-acc-imap-port" class="km-label">IMAP 端口 *</label>
                    <input
                      id="km-acc-imap-port"
                      :value="state.form.value.imapPort"
                      type="number"
                      min="1"
                      max="65535"
                      class="km-input km-mono"
                      @input="onServerPort('imapPort', $event)"
                    />
                    <span v-if="state.validationErrors.value.imapPort" class="km-field-err">
                      {{ state.validationErrors.value.imapPort }}
                    </span>
                  </div>
                  <div class="km-field">
                    <label for="km-acc-imap-sec" class="km-label">IMAP 加密</label>
                    <select
                      id="km-acc-imap-sec"
                      :value="state.form.value.imapSecurity"
                      class="km-input"
                      @change="onServerSecurity('imapSecurity', $event)"
                    >
                      <option value="tls">SSL/TLS</option>
                      <option value="startTls">STARTTLS</option>
                    </select>
                  </div>
                </div>

                <div class="km-grid-server">
                  <div class="km-field">
                    <label for="km-acc-smtp-host" class="km-label">SMTP 服务器 *</label>
                    <input
                      id="km-acc-smtp-host"
                      :value="state.form.value.smtpHost"
                      type="text"
                      class="km-input km-mono"
                      placeholder="smtp.example.com"
                      @input="onServerText('smtpHost', $event)"
                    />
                    <span v-if="state.validationErrors.value.smtpHost" class="km-field-err">
                      {{ state.validationErrors.value.smtpHost }}
                    </span>
                  </div>
                  <div class="km-field">
                    <label for="km-acc-smtp-port" class="km-label">SMTP 端口 *</label>
                    <input
                      id="km-acc-smtp-port"
                      :value="state.form.value.smtpPort"
                      type="number"
                      min="1"
                      max="65535"
                      class="km-input km-mono"
                      @input="onServerPort('smtpPort', $event)"
                    />
                    <span v-if="state.validationErrors.value.smtpPort" class="km-field-err">
                      {{ state.validationErrors.value.smtpPort }}
                    </span>
                  </div>
                  <div class="km-field">
                    <label for="km-acc-smtp-sec" class="km-label">SMTP 加密</label>
                    <select
                      id="km-acc-smtp-sec"
                      :value="state.form.value.smtpSecurity"
                      class="km-input"
                      @change="onServerSecurity('smtpSecurity', $event)"
                    >
                      <option value="tls">SSL/TLS</option>
                      <option value="startTls">STARTTLS</option>
                    </select>
                  </div>
                </div>

                <label class="km-checkbox-row km-separate-toggle">
                  <input v-model="state.form.value.smtpSeparateCredentials" type="checkbox" />
                  <span>使用独立的 SMTP 发信用户名与密码</span>
                </label>

                <div v-if="state.form.value.smtpSeparateCredentials" class="km-grid-2">
                  <div class="km-field">
                    <label for="km-acc-smtp-user" class="km-label">SMTP 用户名 *</label>
                    <input
                      id="km-acc-smtp-user"
                      :value="state.form.value.smtpUsername"
                      type="text"
                      class="km-input"
                      placeholder="smtp-user@example.com"
                      @input="onSmtpUsernameInput"
                    />
                    <span v-if="state.validationErrors.value.smtpUsername" class="km-field-err">
                      {{ state.validationErrors.value.smtpUsername }}
                    </span>
                  </div>

                  <div class="km-field">
                    <label for="km-acc-smtp-pwd" class="km-label">
                      {{
                        state.form.value.id && state.form.value.hasPassword
                          ? "SMTP 密码（留空保留原密码）"
                          : "SMTP 密码 *"
                      }}
                    </label>
                    <input
                      id="km-acc-smtp-pwd"
                      v-model="state.form.value.smtpPassword"
                      type="password"
                      class="km-input"
                      autocomplete="new-password"
                      :placeholder="
                        state.form.value.id && state.form.value.hasPassword
                          ? '已保存密码，留空则保持不变'
                          : '请输入 SMTP 密码'
                      "
                    />
                    <span v-if="state.validationErrors.value.smtpPassword" class="km-field-err">
                      {{ state.validationErrors.value.smtpPassword }}
                    </span>
                  </div>
                </div>
              </template>
            </section>

            <!-- 邮件签名 -->
            <section class="km-form-section">
              <label for="km-acc-signature" class="km-section-title">邮件签名</label>
              <textarea
                id="km-acc-signature"
                v-model="state.form.value.signature"
                class="km-textarea"
                rows="3"
                placeholder="写信或回复时自动附加的默认签名（可选）"
              />
            </section>

            <!-- 状态提示条 -->
            <div
              v-if="state.statusBanner.value"
              class="km-banner"
              :class="`is-${state.statusBanner.value.kind}`"
              role="status"
            >
              {{ state.statusBanner.value.text }}
            </div>

            <!-- 底部操作栏 -->
            <footer class="km-editor-footer">
              <div class="km-footer-left">
                <template v-if="!state.isCreating.value && state.form.value.id">
                  <div
                    v-if="state.confirmDeleteId.value === state.form.value.id"
                    class="km-delete-confirm"
                  >
                    <span class="km-delete-confirm-text">确认永久删除此账号？</span>
                    <button
                      type="button"
                      class="km-btn km-btn-danger"
                      :disabled="state.deletingId.value === state.form.value.id"
                      @click="state.confirmDelete()"
                    >
                      {{ state.deletingId.value ? "删除中…" : "确认删除" }}
                    </button>
                    <button
                      type="button"
                      class="km-btn km-btn-ghost"
                      @click="state.cancelDelete()"
                    >
                      取消
                    </button>
                  </div>
                  <button
                    v-else
                    type="button"
                    class="km-btn km-btn-danger-outline"
                    @click="state.requestDelete(state.form.value.id)"
                  >
                    删除账号
                  </button>
                </template>
              </div>

              <div class="km-footer-right">
                <button
                  type="button"
                  class="km-btn km-btn-secondary"
                  :disabled="state.verifying.value || state.saving.value"
                  @click="state.verifyCurrentAccount()"
                >
                  {{ state.verifying.value ? "正在测试连接…" : "测试连接" }}
                </button>
                <button
                  type="submit"
                  class="km-btn km-btn-primary"
                  :disabled="state.saving.value || state.verifying.value"
                >
                  {{ state.saving.value ? "正在保存…" : "保存账号" }}
                </button>
              </div>
            </footer>
          </form>
        </main>
      </div>
    </section>
  </div>
</template>

<script setup lang="ts">
import { computed, onMounted, onUnmounted, ref, watch } from "vue";
import type { AuthBackend, SecurityKind } from "./types";
import { useAccounts } from "./useAccounts";

const props = withDefaults(
  defineProps<{
    open: boolean;
    initialAccountId?: string | null;
    initialPresetId?: string | null;
  }>(),
  {
    initialAccountId: null,
    initialPresetId: null,
  }
);

const emit = defineEmits<{
  close: [];
  "update:open": [open: boolean];
  changed: [];
  overlayChange: [id: string, open: boolean];
}>();

const OVERLAY_ID = "accounts-modal";
const emailInputRef = ref<HTMLInputElement | null>(null);

const state = useAccounts({
  onChanged() {
    emit("changed");
  },
  onLocked() {
    closeModal();
  },
});

const oauthBannerClass = computed(() => {
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

function avatarInitial(text: string): string {
  const trimmed = text.trim();
  return trimmed ? trimmed.slice(0, 1).toUpperCase() : "@";
}

function onEmailInput(ev: Event) {
  state.updateEmail((ev.target as HTMLInputElement).value);
}

function onPresetChange(ev: Event) {
  const presetId = (ev.target as HTMLSelectElement).value;
  state.selectPreset(presetId, true);
}

function setAuthBackend(backend: AuthBackend) {
  state.form.value.authBackend = backend;
  state.markFieldEdited("authBackend");
}

function onImapUsernameInput(ev: Event) {
  state.form.value.imapUsername = (ev.target as HTMLInputElement).value;
  state.markFieldEdited("imapUsername");
}

function onSmtpUsernameInput(ev: Event) {
  state.form.value.smtpUsername = (ev.target as HTMLInputElement).value;
  state.markFieldEdited("smtpUsername");
}

function onServerText(field: "imapHost" | "smtpHost", ev: Event) {
  state.form.value[field] = (ev.target as HTMLInputElement).value;
  state.markFieldEdited(field);
}

function onServerPort(field: "imapPort" | "smtpPort", ev: Event) {
  const raw = (ev.target as HTMLInputElement).value;
  state.form.value[field] = raw === "" ? Number.NaN : Number(raw);
  state.markFieldEdited(field);
}

function onServerSecurity(field: "imapSecurity" | "smtpSecurity", ev: Event) {
  state.form.value[field] = (ev.target as HTMLSelectElement).value as SecurityKind;
  state.markFieldEdited(field);
}

async function handleSave() {
  await state.saveCurrentAccount();
}

function closeModal() {
  if (!props.open) return;
  emit("overlayChange", OVERLAY_ID, false);
  emit("update:open", false);
  emit("close");
}

function handleKeydown(ev: KeyboardEvent) {
  if (!props.open) return;
  if (ev.key === "Escape") {
    ev.preventDefault();
    ev.stopPropagation();
    closeModal();
  }
}

function attachGlobalListeners() {
  if (typeof window !== "undefined") {
    window.addEventListener("keydown", handleKeydown);
  }
}

function detachGlobalListeners() {
  if (typeof window !== "undefined") {
    window.removeEventListener("keydown", handleKeydown);
  }
}

watch(
  () => props.open,
  (isOpen, prevOpen) => {
    if (isOpen && !prevOpen) {
      emit("overlayChange", OVERLAY_ID, true);
      attachGlobalListeners();
      void state.init({
        initialAccountId: props.initialAccountId,
        initialPresetId: props.initialPresetId,
      });
    } else if (!isOpen && prevOpen) {
      detachGlobalListeners();
      emit("overlayChange", OVERLAY_ID, false);
    }
  }
);

onMounted(() => {
  if (props.open) {
    emit("overlayChange", OVERLAY_ID, true);
    attachGlobalListeners();
    void state.init({
      initialAccountId: props.initialAccountId,
      initialPresetId: props.initialPresetId,
    });
  }
});

onUnmounted(() => {
  detachGlobalListeners();
  if (props.open) {
    emit("overlayChange", OVERLAY_ID, false);
  }
  state.dispose();
});
</script>

<style scoped>
.km-accounts-backdrop {
  position: fixed;
  inset: 0;
  z-index: 1000;
  display: flex;
  align-items: center;
  justify-content: center;
  padding: 16px;
  background: rgba(0, 0, 0, 0.48);
  backdrop-filter: blur(4px);
}

.km-accounts-modal {
  width: min(940px, 100%);
  height: min(680px, calc(100vh - 32px));
  display: flex;
  flex-direction: column;
  border-radius: var(--km-radius-lg);
  border: 1px solid var(--km-border);
  background: var(--km-surface-raised);
  box-shadow: var(--km-shadow-popover);
  overflow: hidden;
}

.km-modal-header {
  height: 52px;
  padding: 0 18px;
  border-bottom: 1px solid var(--km-border);
  background: var(--km-surface);
  display: flex;
  align-items: center;
  justify-content: space-between;
  flex-shrink: 0;
}

.km-modal-title-group {
  display: flex;
  align-items: baseline;
  gap: 10px;
}

.km-modal-title {
  margin: 0;
  font-size: 15px;
  font-weight: 700;
  color: var(--km-text);
}

.km-modal-count {
  font-size: 12px;
  color: var(--km-text-muted);
}

.km-modal-header-actions {
  display: flex;
  align-items: center;
  gap: 8px;
}

.km-close-btn {
  width: 30px;
  height: 30px;
  border: none;
  border-radius: var(--km-radius-sm);
  background: transparent;
  color: var(--km-text-muted);
  font-size: 14px;
  cursor: pointer;
}

.km-close-btn:hover {
  background: var(--km-hover-bg);
  color: var(--km-text);
}

.km-modal-body {
  flex: 1;
  min-height: 0;
  display: flex;
}

/* 左侧账号列表 */
.km-account-sidebar {
  width: 268px;
  min-width: 240px;
  border-right: 1px solid var(--km-border);
  background: var(--km-sidebar);
  overflow-y: auto;
  padding: 10px;
}

.km-sidebar-empty,
.km-sidebar-error {
  padding: 24px 12px;
  text-align: center;
  color: var(--km-text-muted);
  font-size: 12px;
  display: flex;
  flex-direction: column;
  align-items: center;
  gap: 10px;
}

.km-sidebar-error {
  color: var(--km-danger);
}

.km-account-list {
  list-style: none;
  margin: 0;
  padding: 0;
  display: flex;
  flex-direction: column;
  gap: 6px;
}

.km-account-item {
  display: flex;
  align-items: center;
  gap: 8px;
  padding: 6px 8px;
  border-radius: var(--km-radius-md);
  border: 1px solid transparent;
  background: transparent;
}

.km-account-item:hover {
  background: var(--km-hover-bg);
}

.km-account-item.active {
  background: var(--km-surface);
  border-color: var(--km-accent);
  box-shadow: var(--km-shadow-card);
}

.km-account-item.disabled .km-acc-info {
  opacity: 0.6;
}

.km-account-select-btn {
  flex: 1;
  min-width: 0;
  border: none;
  background: transparent;
  padding: 0;
  display: flex;
  align-items: center;
  gap: 9px;
  text-align: left;
  cursor: pointer;
}

.km-acc-avatar {
  width: 30px;
  height: 30px;
  border-radius: 999px;
  background: var(--km-accent-soft);
  color: var(--km-accent);
  font-weight: 700;
  font-size: 13px;
  display: inline-flex;
  align-items: center;
  justify-content: center;
  flex-shrink: 0;
}

.km-acc-info {
  flex: 1;
  min-width: 0;
  display: flex;
  flex-direction: column;
}

.km-acc-name-row {
  display: flex;
  align-items: center;
  gap: 5px;
}

.km-acc-name {
  font-size: 12px;
  font-weight: 600;
  color: var(--km-text);
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.km-acc-tag {
  padding: 0 5px;
  border-radius: 4px;
  font-size: 10px;
  font-weight: 600;
  background: var(--km-accent-soft);
  color: var(--km-accent);
  flex-shrink: 0;
}

.km-acc-email {
  font-size: 11px;
  color: var(--km-text-muted);
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.km-switch {
  position: relative;
  width: 32px;
  height: 18px;
  border-radius: 999px;
  border: none;
  background: color-mix(in srgb, var(--km-text-muted) 35%, transparent);
  padding: 2px;
  cursor: pointer;
  flex-shrink: 0;
  transition: background-color 0.15s ease;
}

.km-switch.checked {
  background: var(--km-accent);
}

.km-switch-thumb {
  display: block;
  width: 14px;
  height: 14px;
  border-radius: 999px;
  background: #ffffff;
  transition: transform 0.15s ease;
}

.km-switch.checked .km-switch-thumb {
  transform: translateX(14px);
}

/* 右侧表单 */
.km-account-editor {
  flex: 1;
  min-width: 0;
  overflow-y: auto;
  padding: 18px 22px;
  background: var(--km-surface-raised);
}

.km-editor-form {
  display: flex;
  flex-direction: column;
  gap: 16px;
}

.km-editor-header {
  display: flex;
  align-items: flex-start;
  justify-content: space-between;
  gap: 14px;
  flex-wrap: wrap;
}

.km-editor-title {
  margin: 0;
  font-size: 15px;
  font-weight: 700;
  color: var(--km-text);
}

.km-editor-subtitle {
  margin: 2px 0 0;
  font-size: 12px;
  color: var(--km-text-muted);
}

.km-preset-picker {
  display: flex;
  align-items: center;
  gap: 6px;
}

.km-preset-select {
  min-width: 190px;
}

.km-reset-btn {
  font-size: 12px;
  padding: 0 8px;
}

.km-note-banner {
  padding: 9px 12px;
  border-radius: var(--km-radius-sm);
  border: 1px solid color-mix(in srgb, var(--km-accent) 28%, var(--km-border));
  background: var(--km-accent-soft);
  font-size: 12px;
  color: var(--km-text);
}

.km-form-section {
  display: flex;
  flex-direction: column;
  gap: 10px;
  padding-top: 10px;
  border-top: 1px solid var(--km-border);
}

.km-section-header-row {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 10px;
  flex-wrap: wrap;
}

.km-section-title {
  margin: 0;
  font-size: 12px;
  font-weight: 700;
  color: var(--km-text-muted);
  text-transform: uppercase;
  letter-spacing: 0.02em;
}

.km-auth-tabs {
  display: inline-flex;
  border: 1px solid var(--km-border);
  border-radius: var(--km-radius-sm);
  overflow: hidden;
  background: var(--km-canvas);
}

.km-auth-tab {
  border: none;
  background: transparent;
  padding: 5px 10px;
  font-size: 12px;
  cursor: pointer;
  color: var(--km-text-muted);
}

.km-auth-tab.active {
  background: var(--km-accent);
  color: var(--km-accent-fg);
  font-weight: 600;
}

.km-grid-2 {
  display: grid;
  grid-template-columns: repeat(2, minmax(0, 1fr));
  gap: 12px;
}

.km-grid-server {
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

.km-input,
.km-textarea {
  width: 100%;
  border: 1px solid var(--km-border);
  border-radius: var(--km-radius-sm);
  background: var(--km-canvas);
  color: var(--km-text);
  user-select: text;
}

.km-input {
  height: 32px;
  padding: 0 10px;
}

.km-textarea {
  padding: 8px 10px;
  resize: vertical;
  min-height: 68px;
}

.km-input:focus,
.km-textarea:focus {
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

.km-toggles-col {
  justify-content: center;
  gap: 6px;
}

.km-checkbox-row {
  display: inline-flex;
  align-items: center;
  gap: 7px;
  font-size: 12px;
  color: var(--km-text);
  cursor: pointer;
}

.km-separate-toggle {
  margin-top: 2px;
}

.km-oauth-box {
  display: flex;
  flex-direction: column;
  gap: 10px;
  padding: 12px;
  border-radius: var(--km-radius-md);
  border: 1px solid var(--km-border);
  background: var(--km-canvas);
}

.km-oauth-top {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 12px;
  flex-wrap: wrap;
}

.km-oauth-title {
  font-size: 13px;
  font-weight: 600;
  color: var(--km-text);
}

.km-oauth-desc {
  margin-top: 2px;
  font-size: 12px;
  color: var(--km-text-muted);
}

.km-banner {
  padding: 9px 12px;
  border-radius: var(--km-radius-sm);
  font-size: 12px;
  border: 1px solid var(--km-border);
  background: var(--km-canvas);
}

.km-banner.is-success {
  border-color: color-mix(in srgb, #26a269 45%, var(--km-border));
  background: color-mix(in srgb, #26a269 12%, transparent);
}

.km-banner.is-error {
  border-color: color-mix(in srgb, var(--km-danger) 50%, var(--km-border));
  background: var(--km-danger-soft);
  color: var(--km-danger);
}

.km-banner.is-info {
  border-color: color-mix(in srgb, var(--km-accent) 45%, var(--km-border));
  background: var(--km-accent-soft);
}

.km-editor-footer {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 12px;
  padding-top: 12px;
  border-top: 1px solid var(--km-border);
  flex-wrap: wrap;
}

.km-footer-left,
.km-footer-right {
  display: flex;
  align-items: center;
  gap: 8px;
}

.km-delete-confirm {
  display: flex;
  align-items: center;
  gap: 6px;
}

.km-delete-confirm-text {
  font-size: 12px;
  font-weight: 600;
  color: var(--km-danger);
}

.km-btn {
  height: 32px;
  padding: 0 12px;
  border-radius: var(--km-radius-sm);
  border: 1px solid var(--km-border);
  font-size: 12px;
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
  color: var(--km-accent);
}

.km-btn-ghost:hover:not(:disabled) {
  background: var(--km-hover-bg);
}

.km-btn-danger-outline {
  border-color: color-mix(in srgb, var(--km-danger) 50%, var(--km-border));
  background: transparent;
  color: var(--km-danger);
}

.km-btn-danger-outline:hover:not(:disabled) {
  background: var(--km-danger-soft);
}

.km-btn-danger {
  border-color: var(--km-danger);
  background: var(--km-danger);
  color: #ffffff;
}

.km-sr-only {
  position: absolute;
  width: 1px;
  height: 1px;
  overflow: hidden;
  clip: rect(0 0 0 0);
}

@media (max-width: 760px) {
  .km-modal-body {
    flex-direction: column;
  }

  .km-account-sidebar {
    width: 100%;
    max-height: 180px;
    border-right: none;
    border-bottom: 1px solid var(--km-border);
  }

  .km-grid-2,
  .km-grid-server {
    grid-template-columns: 1fr;
  }
}
</style>
