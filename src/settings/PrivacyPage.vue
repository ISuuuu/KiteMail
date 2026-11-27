<template>
  <div class="km-sp-page">
    <!-- 页级反馈（锁屏表单关闭时显示，对齐 GPUI app_lock_feedback） -->
    <div
      v-if="pageFeedback && lockFormMode === 'closed'"
      class="km-sp-feedback"
      :class="{ error: !pageFeedback.ok }"
      role="status"
    >
      {{ pageFeedback.text }}
    </div>

    <!-- 分组一：应用锁屏密码 -->
    <section class="km-sp-group">
      <h4 class="km-sp-title">应用锁屏密码</h4>
      <p class="km-sp-desc">
        设置本地密码后，无操作或切到后台超过设定时间将自动锁定界面；锁定状态下重启软件仍需输入密码解锁
      </p>
      <div class="km-sp-card">
        <div class="km-sp-row" id="sp-app-lock-status">
          <div class="km-sp-row-text">
            <span class="km-sp-row-title">锁屏密码</span>
            <span class="km-sp-row-desc">
              {{
                state.hasPasscode.value
                  ? "已启用 · 每次重新打开软件、空闲或切到后台超时后需输入密码解锁"
                  : "未设置 · 设置密码后将默认开启自动锁定与启动锁屏保护"
              }}
            </span>
          </div>
          <div class="km-sp-row-control km-sp-lock-actions">
            <template v-if="state.hasPasscode.value">
              <button
                type="button"
                class="km-sp-outline-btn"
                :disabled="state.passcodeBusy.value"
                @click="lockNow"
              >
                立即锁定
              </button>
              <button
                type="button"
                class="km-sp-outline-btn"
                :disabled="state.passcodeBusy.value"
                @click="openLockForm('change')"
              >
                修改密码
              </button>
              <button
                type="button"
                class="km-sp-outline-btn km-sp-danger-btn"
                :disabled="state.passcodeBusy.value"
                @click="openLockForm('disable')"
              >
                关闭密码
              </button>
            </template>
            <button
              v-else
              type="button"
              class="km-sp-primary-btn"
              :disabled="state.passcodeBusy.value"
              @click="openLockForm('setup')"
            >
              设置密码
            </button>
          </div>
        </div>

        <!-- 内联锁屏密码表单（对齐 GPUI AppLockFormMode） -->
        <template v-if="lockFormMode !== 'closed'">
          <div class="km-sp-sep" />
          <div class="km-lock-form">
            <div class="km-sp-row-text">
              <span class="km-lock-form-title">{{ lockFormMeta.title }}</span>
              <span class="km-sp-row-desc">{{ lockFormMeta.hint }}</span>
            </div>

            <label v-if="lockFormMode !== 'setup'" class="km-lock-field">
              <span class="km-lock-field-label">当前密码</span>
              <input
                v-model="currentPasscode"
                type="password"
                class="km-sp-input"
                autocomplete="current-password"
              />
            </label>
            <label v-if="lockFormMode !== 'disable'" class="km-lock-field">
              <span class="km-lock-field-label">新密码</span>
              <input
                v-model="newPasscode"
                type="password"
                class="km-sp-input"
                autocomplete="new-password"
              />
            </label>
            <label v-if="lockFormMode !== 'disable'" class="km-lock-field">
              <span class="km-lock-field-label">确认新密码</span>
              <input
                v-model="confirmPasscode"
                type="password"
                class="km-sp-input"
                autocomplete="new-password"
              />
            </label>

            <div
              v-if="state.passcodeError.value"
              class="km-sp-feedback error"
              role="alert"
            >
              {{ state.passcodeError.value }}
            </div>

            <div class="km-lock-form-actions">
              <button
                type="button"
                class="km-sp-outline-btn"
                :disabled="state.passcodeBusy.value"
                @click="closeLockForm"
              >
                取消
              </button>
              <button
                type="button"
                :class="lockFormMode === 'disable' ? 'km-sp-danger-btn' : 'km-sp-primary-btn'"
                class="km-sp-submit-btn"
                :disabled="state.passcodeBusy.value"
                @click="submitLockForm"
              >
                {{ lockFormMeta.submitLabel }}
              </button>
            </div>
          </div>
        </template>

        <div class="km-sp-sep" />
        <div class="km-sp-row" :class="{ disabled: !state.hasPasscode.value }" id="sp-app-lock-idle">
          <div class="km-sp-row-text">
            <span class="km-sp-row-title">无操作自动锁定</span>
            <span class="km-sp-row-desc">连续未操作鼠标或键盘达到设定时长后自动锁定界面</span>
          </div>
          <div class="km-sp-row-control">
            <ComboSelect
              :options="APP_LOCK_IDLE_OPTIONS"
              :modelValue="String(state.preferences.value.appLockIdleMinutes)"
              :disabled="!state.hasPasscode.value"
              title="无操作自动锁定"
              @update:modelValue="setIdleMinutes"
            />
          </div>
        </div>
        <div class="km-sp-sep" />
        <div class="km-sp-row" :class="{ disabled: !state.hasPasscode.value }" id="sp-app-lock-background">
          <div class="km-sp-row-text">
            <span class="km-sp-row-title">切到后台或最小化后自动锁定</span>
            <span class="km-sp-row-desc">窗口切到后台、最小化或隐藏到托盘超过设定时长后自动锁定</span>
          </div>
          <div class="km-sp-row-control">
            <ComboSelect
              :options="APP_LOCK_BG_OPTIONS"
              :modelValue="bgTimeout"
              :disabled="!state.hasPasscode.value"
              title="切到后台或最小化后自动锁定"
              @update:modelValue="bgTimeout = $event"
            />
          </div>
        </div>
        <div class="km-sp-sep" />
        <div class="km-sp-row" :class="{ disabled: !state.hasPasscode.value }" id="sp-app-lock-system">
          <div class="km-sp-row-text">
            <span class="km-sp-row-title">系统锁屏或休眠时立即锁定</span>
            <span class="km-sp-row-desc">触发 Windows 锁屏（Win+L）、切换用户或电脑睡眠时立刻锁定应用</span>
          </div>
          <div class="km-sp-row-control">
            <PillSwitch
              v-model="appLockOnSystemLock"
              :disabled="!state.hasPasscode.value"
            />
          </div>
        </div>
      </div>
    </section>

    <!-- 分组二：远程图像与跟踪保护 -->
    <section class="km-sp-group">
      <h4 class="km-sp-title">远程图像与跟踪保护</h4>
      <div class="km-sp-card">
        <div class="km-sp-row" id="sp-blockRemoteImages">
          <div class="km-sp-row-text">
            <span class="km-sp-row-title">默认屏蔽远程图像</span>
            <span class="km-sp-row-desc">阻止外部图片链接追踪您的阅读时间与 IP</span>
          </div>
          <div class="km-sp-row-control">
            <PillSwitch
              :modelValue="state.preferences.value.blockRemoteImages"
              @update:modelValue="setBoolPref('blockRemoteImages', $event)"
            />
          </div>
        </div>
        <div class="km-sp-sep" />
        <div class="km-sp-row" id="sp-showRemoteBanner">
          <div class="km-sp-row-text">
            <span class="km-sp-row-title">显示已屏蔽远程图像提示横幅</span>
            <span class="km-sp-row-desc">关闭后静默屏蔽外链图片，不再显示提示横幅</span>
          </div>
          <div class="km-sp-row-control">
            <PillSwitch
              :modelValue="state.preferences.value.showRemoteBanner"
              @update:modelValue="setBoolPref('showRemoteBanner', $event)"
            />
          </div>
        </div>
        <div class="km-sp-sep" />
        <div class="km-sp-row" id="sp-cleanLinks">
          <div class="km-sp-row-text">
            <span class="km-sp-row-title">自动剥离链接中的追踪参数</span>
            <span class="km-sp-row-desc">渲染邮件链接时移除 utm_* / fbclid / gclid / msclkid / spm 等参数</span>
          </div>
          <div class="km-sp-row-control">
            <PillSwitch
              :modelValue="state.preferences.value.cleanLinks"
              @update:modelValue="setBoolPref('cleanLinks', $event)"
            />
          </div>
        </div>
        <div class="km-sp-sep" />
        <div class="km-sp-row" id="sp-showSpoofBanner">
          <div class="km-sp-row-text">
            <span class="km-sp-row-title">警告仿冒域名发件人</span>
            <span class="km-sp-row-desc">检测到显示名称或域名疑似钓鱼仿冒时在阅读器顶部显示警告横幅</span>
          </div>
          <div class="km-sp-row-control">
            <PillSwitch v-model="privacyState.showSpoofBanner" />
          </div>
        </div>
      </div>
    </section>

    <!-- 分组三：头像与公司徽标 -->
    <section class="km-sp-group">
      <h4 class="km-sp-title">头像与公司徽标</h4>
      <p class="km-sp-desc">
        从第三方服务拉取发件人头像或品牌徽标（可能向对应服务发送邮箱哈希或域名请求）
      </p>
      <div class="km-sp-card">
        <div class="km-sp-row" id="sp-useGravatar">
          <div class="km-sp-row-text">
            <span class="km-sp-row-title">加载 Gravatar 头像</span>
            <span class="km-sp-row-desc">根据发件人邮箱哈希请求 Gravatar 公开头像</span>
          </div>
          <div class="km-sp-row-control">
            <PillSwitch v-model="privacyState.useGravatar" />
          </div>
        </div>
        <div class="km-sp-sep" />
        <div class="km-sp-row" id="sp-senderLogos">
          <div class="km-sp-row-text">
            <span class="km-sp-row-title">加载公司徽标</span>
            <span class="km-sp-row-desc">按发件人域名展示对应的品牌或企业图标</span>
          </div>
          <div class="km-sp-row-control">
            <PillSwitch v-model="privacyState.senderLogos" />
          </div>
        </div>
      </div>
    </section>

    <!-- 分组四：已读回执 -->
    <section class="km-sp-group">
      <h4 class="km-sp-title">已读回执</h4>
      <div class="km-sp-card">
        <div class="km-sp-row" id="sp-read-receipt">
          <div class="km-sp-row-text">
            <span class="km-sp-row-title">发送已读回执</span>
            <span class="km-sp-row-desc">当发件人请求阅读回执（Disposition-Notification-To）时的处理策略</span>
          </div>
          <div class="km-sp-row-control">
            <ComboSelect
              :options="READ_RECEIPT_OPTIONS"
              :modelValue="readReceiptPolicy"
              title="发送已读回执"
              @update:modelValue="readReceiptPolicy = $event"
            />
          </div>
        </div>
      </div>
    </section>
  </div>
</template>

<script setup lang="ts">
import "./page.css";
import { computed, reactive, ref } from "vue";
import { lockApp } from "../ipc/main-window";
import ComboSelect from "./ComboSelect.vue";
import PillSwitch from "./PillSwitch.vue";
import type { FullPreferencesDto } from "../preferences/types";
import type { PrefsPanelState } from "../preferences/panels/state";

/**
 * 「隐私」页（对齐 GPUI pages.rs#render_privacy）。
 * 锁屏密码（设置/修改/关闭/立即锁定/无操作自动锁定）直连既有
 * set_app_lock_passcode / lock_app IPC；后台锁定超时、系统锁屏联动、
 * 仿冒警告、头像徽标与已读回执为会话内状态。
 */
const props = defineProps<{ state: PrefsPanelState }>();

// 模板与表单元数据共用同一个状态引用。
const state = props.state;

type LockFormMode = "closed" | "setup" | "change" | "disable";

const APP_LOCK_IDLE_OPTIONS = [
  { label: "1 分钟", value: "1" },
  { label: "5 分钟", value: "5" },
  { label: "10 分钟", value: "10" },
  { label: "15 分钟", value: "15" },
  { label: "30 分钟", value: "30" },
  { label: "60 分钟", value: "60" },
  { label: "从不", value: "0" },
] as const;

const APP_LOCK_BG_OPTIONS = [
  { label: "立即", value: "immediate" },
  { label: "1 分钟", value: "1" },
  { label: "3 分钟", value: "3" },
  { label: "5 分钟", value: "5" },
  { label: "10 分钟", value: "10" },
  { label: "15 分钟", value: "15" },
  { label: "30 分钟", value: "30" },
  { label: "从不", value: "never" },
] as const;

const READ_RECEIPT_OPTIONS = [
  { label: "每次询问", value: "ask" },
  { label: "总是发送", value: "always" },
  { label: "从不发送", value: "never" },
] as const;

const lockFormMode = ref<LockFormMode>("closed");
const currentPasscode = ref("");
const newPasscode = ref("");
const confirmPasscode = ref("");
const pageFeedback = ref<{ ok: boolean; text: string } | null>(null);

const lockFormMeta = computed(() => {
  switch (lockFormMode.value) {
    case "setup":
      return {
        title: "设置锁屏密码",
        hint: "密码至少需要 4 个字符；保存后将默认开启空闲 15 分钟 / 后台 5 分钟自动锁定。",
        submitLabel: state.passcodeBusy.value ? "保存中..." : "保存密码",
      };
    case "change":
      return {
        title: "修改锁屏密码",
        hint: "请先验证当前密码，再输入并确认新密码（至少 4 个字符）。",
        submitLabel: state.passcodeBusy.value ? "保存中..." : "确认修改",
      };
    case "disable":
      return {
        title: "关闭锁屏密码",
        hint: "关闭前需验证当前密码；关闭后重启与自动锁屏将不再要求输入密码。",
        submitLabel: state.passcodeBusy.value ? "处理中..." : "确认关闭",
      };
    default:
      return { title: "", hint: "", submitLabel: "" };
  }
});


function clearFormInputs() {
  currentPasscode.value = "";
  newPasscode.value = "";
  confirmPasscode.value = "";
}

function openLockForm(mode: Exclude<LockFormMode, "closed">) {
  lockFormMode.value = mode;
  pageFeedback.value = null;
  clearFormInputs();
}

function closeLockForm() {
  lockFormMode.value = "closed";
  pageFeedback.value = null;
  clearFormInputs();
}

async function submitLockForm() {
  pageFeedback.value = null;
  // 先捕获模式：closeLockForm 后 lockFormMode 已重置。
  const mode = lockFormMode.value;
  if (mode === "disable") {
    const ok = await props.state.clearPasscode(currentPasscode.value);
    if (ok) {
      closeLockForm();
      pageFeedback.value = { ok: true, text: "锁屏密码已关闭" };
    }
    return;
  }
  const ok = await props.state.submitPasscode({
    currentPasscode: props.state.hasPasscode.value ? currentPasscode.value : undefined,
    newPasscode: newPasscode.value,
    confirmPasscode: confirmPasscode.value,
  });
  if (ok) {
    closeLockForm();
    pageFeedback.value = {
      ok: true,
      text: mode === "setup" ? "锁屏密码已设置" : "锁屏密码已修改",
    };
  }
}

async function lockNow() {
  try {
    await lockApp();
  } catch {
    pageFeedback.value = { ok: false, text: "立即锁定失败，请稍后重试" };
  }
}

function setIdleMinutes(value: string) {
  const num = Number(value);
  if (!Number.isFinite(num)) return;
  void props.state.updatePreference({ appLockIdleMinutes: num });
}

function setBoolPref(
  field: "blockRemoteImages" | "showRemoteBanner" | "cleanLinks",
  value: boolean
) {
  void props.state.updatePreference({ [field]: value } as Partial<FullPreferencesDto>);
}

// ---- 会话内状态（对齐 mail-core 字段默认值；回执策略为独立字符串项） ----
const bgTimeout = ref("5");
const appLockOnSystemLock = ref(true);
const readReceiptPolicy = ref("ask");
const privacyState = reactive<Record<string, boolean>>({
  showSpoofBanner: true,
  useGravatar: true,
  senderLogos: true,
});
</script>

<style scoped>
.km-sp-lock-actions {
  gap: 6px;
  flex-wrap: wrap;
  justify-content: flex-end;
}

.km-sp-danger-btn {
  border-color: color-mix(in srgb, var(--km-danger) 50%, var(--km-border));
  color: var(--km-danger);
}

.km-sp-danger-btn:hover:not(:disabled) {
  background: var(--km-danger-soft);
}

.km-sp-feedback.error {
  background: var(--km-danger-soft);
  color: var(--km-danger);
}

.km-lock-form {
  padding: 12px 16px;
  display: flex;
  flex-direction: column;
  gap: 10px;
}

.km-lock-form-title {
  font-size: 13px;
  font-weight: 600;
  color: var(--km-text);
}

.km-lock-field {
  display: flex;
  flex-direction: column;
  gap: 4px;
}

.km-lock-field-label {
  font-size: 12px;
  font-weight: 500;
  color: var(--km-text-muted);
}

.km-lock-form-actions {
  display: flex;
  justify-content: flex-end;
  gap: 8px;
  padding-top: 2px;
}

.km-sp-submit-btn {
  min-width: 88px;
}
</style>
