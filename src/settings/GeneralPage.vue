<template>
  <div class="km-general-page">
    <!-- 分组一：分页加载 -->
    <section class="km-gen-group">
      <h4 class="km-gen-group-title">分页加载</h4>
      <div class="km-gen-card">
        <div class="km-gen-row">
          <div class="km-gen-row-text">
            <span class="km-gen-row-title">单页加载邮件数</span>
          </div>
          <div class="km-gen-row-control">
            <ComboSelect
              :options="PAGE_SIZE_OPTIONS"
              :modelValue="String(pageSize)"
              title="单页加载邮件数"
              @update:modelValue="pageSize = Number($event)"
            />
          </div>
        </div>
      </div>
    </section>

    <!-- 分组二：同步与通知 -->
    <section class="km-gen-group">
      <h4 class="km-gen-group-title">同步与通知</h4>
      <div class="km-gen-card">
        <div class="km-gen-row">
          <div class="km-gen-row-text">
            <span class="km-gen-row-title">自动后台同步间隔</span>
            <span class="km-gen-row-desc">按设定分钟轮询所有已配置邮箱的新邮件</span>
          </div>
          <div class="km-gen-row-control">
            <ComboSelect
              :options="SYNC_INTERVAL_OPTIONS"
              :modelValue="String(state.preferences.value.autoSyncMinutes)"
              title="自动后台同步间隔"
              @update:modelValue="setSyncInterval"
            />
          </div>
        </div>

        <div class="km-gen-sep" />

        <div class="km-gen-row">
          <div class="km-gen-row-text">
            <span class="km-gen-row-title">启用实时推送（IMAP IDLE / 增量脉冲）</span>
            <span class="km-gen-row-desc">在后台维持实时监听连接以便秒级感知新邮件到达</span>
          </div>
          <div class="km-gen-row-control">
            <PillSwitch
              :modelValue="state.preferences.value.pushEnabled"
              @update:modelValue="setBoolPref('pushEnabled', $event)"
            />
          </div>
        </div>

        <div class="km-gen-sep" />

        <div class="km-gen-row">
          <div class="km-gen-row-text">
            <span class="km-gen-row-title">按流量计费连接省流</span>
            <span class="km-gen-row-desc">使用按流量计费的网络时仅收取新邮件，不预取其余内容</span>
          </div>
          <div class="km-gen-row-control">
            <!-- 暂未进入偏好 IPC 契约，先按旧版呈现可选状态（会话内生效） -->
            <PillSwitch v-model="meteredSaveData" />
          </div>
        </div>

        <div class="km-gen-sep" />

        <div class="km-gen-row">
          <div class="km-gen-row-text">
            <span class="km-gen-row-title">新邮件桌面通知</span>
            <span class="km-gen-row-desc">同步发现新增未读邮件时通过 Windows 托盘弹出提示</span>
          </div>
          <div class="km-gen-row-control">
            <PillSwitch
              :modelValue="state.preferences.value.desktopNotifications"
              @update:modelValue="setBoolPref('desktopNotifications', $event)"
            />
          </div>
        </div>

        <div class="km-gen-sep" />

        <div class="km-gen-row" :class="{ disabled: !state.preferences.value.desktopNotifications }">
          <div class="km-gen-row-text">
            <span class="km-gen-row-title">在桌面通知中显示发件人与主题</span>
            <span class="km-gen-row-desc">关闭后通知仅提示「收到新邮件 / 封数」，保护锁屏及旁观隐私</span>
          </div>
          <div class="km-gen-row-control">
            <PillSwitch
              :modelValue="state.preferences.value.notificationShowContent"
              :disabled="!state.preferences.value.desktopNotifications"
              @update:modelValue="setBoolPref('notificationShowContent', $event)"
            />
          </div>
        </div>

        <div class="km-gen-sep" />

        <div class="km-gen-row" :class="{ disabled: !state.preferences.value.desktopNotifications }">
          <div class="km-gen-row-text">
            <span class="km-gen-row-title">新邮件提示音</span>
            <span class="km-gen-row-desc">收到新邮件时随通知播放提示音（音频后端暂缺，先保存选择）</span>
          </div>
          <div class="km-gen-row-control">
            <PillSwitch
              v-model="notificationSoundEnabled"
              :disabled="!state.preferences.value.desktopNotifications"
            />
          </div>
        </div>

        <div class="km-gen-sep" />

        <div
          class="km-gen-row"
          :class="{ disabled: !state.preferences.value.desktopNotifications }"
        >
          <div class="km-gen-row-text">
            <span class="km-gen-row-title">提示音选择</span>
            <span class="km-gen-row-desc">内置 4 种提示音（对齐 Linux 版）</span>
          </div>
          <div class="km-gen-row-control km-gen-sound-control">
            <ComboSelect
              :options="NOTIFICATION_SOUND_OPTIONS"
              :modelValue="notificationSound"
              :disabled="
                !state.preferences.value.desktopNotifications || !notificationSoundEnabled
              "
              title="提示音选择"
              @update:modelValue="notificationSound = $event"
            />
            <!-- 音频后端暂未接入：试听恒为禁用态（对齐 GPUI btn-sound-preview disabled(true)） -->
            <button
              type="button"
              class="km-gen-preview-btn"
              title="试听（音频后端暂未接入）"
              disabled
            >
              <SettingsIcon name="play" :size="12" />
            </button>
          </div>
        </div>

        <div class="km-gen-sep" />

        <div class="km-gen-row">
          <div class="km-gen-row-text">
            <span class="km-gen-row-title">发送邮件提示音</span>
            <span class="km-gen-row-desc">邮件发出时播放提示音（音频后端暂缺，先保存选择）</span>
          </div>
          <div class="km-gen-row-control">
            <!-- 暂未进入偏好 IPC 契约，先按旧版呈现可选状态（会话内生效） -->
            <PillSwitch v-model="sendSoundEnabled" />
          </div>
        </div>
      </div>
    </section>

    <!-- 分组三：通知操作按钮（6 选 3，选满后其余置灰） -->
    <section class="km-gen-group">
      <h4 class="km-gen-group-title">通知操作按钮</h4>
      <p class="km-gen-group-desc">
        单封邮件的通知最多显示 3 个操作按钮，超过后其余按钮置灰（通知按钮的实际渲染随后续通知工作落地）
      </p>
      <div class="km-gen-card">
        <template v-for="(btn, idx) in NOTIFICATION_BUTTONS" :key="btn.key">
          <div v-if="idx > 0" class="km-gen-sep" />
          <div
            class="km-gen-row"
            :class="{ disabled: !state.preferences.value.desktopNotifications || !rowSensitive(btn.key) }"
          >
            <div class="km-gen-row-text">
              <span class="km-gen-row-title">{{ btn.label }}</span>
            </div>
            <div class="km-gen-row-control">
              <PillSwitch
                :modelValue="notificationButtons[btn.key]"
                :disabled="
                  !state.preferences.value.desktopNotifications || !rowSensitive(btn.key)
                "
                @update:modelValue="setNotificationButton(btn.key, $event)"
              />
            </div>
          </div>
        </template>
      </div>
    </section>
  </div>
</template>

<script setup lang="ts">
import { reactive, ref } from "vue";
import ComboSelect from "./ComboSelect.vue";
import PillSwitch from "./PillSwitch.vue";
import SettingsIcon from "./SettingsIcon.vue";
import type { FullPreferencesDto } from "../preferences/types";
import type { PrefsPanelState } from "../preferences/panels/state";

/**
 * 「常规」页（对齐 GPUI pages.rs#render_general + widgets.rs#setting_row）：
 * 三组（分页加载 / 同步与通知 / 通知操作按钮），分组标题与说明置于卡片外，
 * 普通组最大宽 600px，行最小高 52px，左侧标题与说明、右侧控件。
 * 已进入 IPC 契约的字段直连 usePreferences；其余字段按旧版呈现（会话内生效）。
 */
const props = defineProps<{ state: PrefsPanelState }>();

const PAGE_SIZE_OPTIONS = [
  { label: "50 封 / 页", value: "50" },
  { label: "100 封 / 页", value: "100" },
  { label: "200 封 / 页", value: "200" },
] as const;

const SYNC_INTERVAL_OPTIONS = [
  { label: "仅手动同步", value: "0" },
  { label: "每 1 分钟", value: "1" },
  { label: "每 5 分钟", value: "5" },
  { label: "每 15 分钟", value: "15" },
  { label: "每 30 分钟", value: "30" },
] as const;

const NOTIFICATION_SOUND_OPTIONS = [
  { label: "点击 (Click)", value: "click" },
  { label: "低鸣 (Hum)", value: "hum" },
  { label: "弦音 (String)", value: "string" },
  { label: "摇摆 (Swing)", value: "swing" },
] as const;

const NOTIFICATION_BUTTONS = [
  { key: "mark_read", label: "标记为已读" },
  { key: "archive", label: "归档" },
  { key: "delete", label: "删除" },
  { key: "reply", label: "回复" },
  { key: "forward", label: "转发" },
  { key: "spam", label: "标记为垃圾邮件" },
] as const;

type NotificationButtonKey = (typeof NOTIFICATION_BUTTONS)[number]["key"];

// ---- 尚未进入 FullPreferencesDto 的字段（对齐 mail-core 默认值，会话内生效） ----
const pageSize = ref(100);
const meteredSaveData = ref(false);
const notificationSoundEnabled = ref(true);
const notificationSound = ref<string>("click");
const sendSoundEnabled = ref(true);
const notificationButtons = reactive<Record<NotificationButtonKey, boolean>>({
  mark_read: true,
  archive: true,
  delete: true,
  reply: false,
  forward: false,
  spam: false,
});

const NOTIFICATION_BUTTONS_MAX = 3;

function selectedCount(): number {
  return NOTIFICATION_BUTTONS.filter((b) => notificationButtons[b.key]).length;
}

/** 已开启的随时可关；未开启的需未满 3 项（对齐 NotificationButtons::row_sensitive）。 */
function rowSensitive(key: NotificationButtonKey): boolean {
  return notificationButtons[key] || selectedCount() < NOTIFICATION_BUTTONS_MAX;
}

function setNotificationButton(key: NotificationButtonKey, value: boolean) {
  notificationButtons[key] = value;
}

function setBoolPref(field: "pushEnabled" | "desktopNotifications" | "notificationShowContent", value: boolean) {
  void props.state.updatePreference({ [field]: value } as Partial<FullPreferencesDto>);
}

function setSyncInterval(value: string) {
  const num = Number(value);
  if (!Number.isFinite(num)) return;
  void props.state.updatePreference({ autoSyncMinutes: num });
}
</script>

<style scoped>
.km-general-page {
  display: flex;
  flex-direction: column;
  align-items: center;
  gap: 20px;
  width: 100%;
}

.km-gen-group {
  display: flex;
  flex-direction: column;
  gap: 6px;
  width: 100%;
  max-width: 600px;
}

.km-gen-group-title {
  margin: 0;
  padding: 0 4px;
  font-size: 14px;
  font-weight: 600;
  color: var(--km-text);
}

.km-gen-group-desc {
  margin: 0;
  padding: 0 4px;
  font-size: 12px;
  color: var(--km-text-muted);
}

.km-gen-card {
  background: var(--km-surface-raised);
  border: 1px solid var(--km-border);
  border-radius: 12px;
  overflow: hidden;
  width: 100%;
}

.km-gen-sep {
  width: 100%;
  height: 1px;
  background: var(--km-border);
  flex-shrink: 0;
}

.km-gen-row {
  width: 100%;
  min-height: 52px;
  padding: 10px 16px;
  gap: 12px;
  display: flex;
  align-items: center;
  justify-content: space-between;
}

.km-gen-row-text {
  flex: 1;
  min-width: 0;
  display: flex;
  flex-direction: column;
  gap: 2px;
}

.km-gen-row-title {
  font-size: 13px;
  font-weight: 500;
  color: var(--km-text);
  word-break: break-word;
}

.km-gen-row-desc {
  font-size: 12px;
  color: var(--km-text-muted);
  word-break: break-word;
}

.km-gen-row.disabled .km-gen-row-title,
.km-gen-row.disabled .km-gen-row-desc {
  color: var(--km-text-muted);
}

.km-gen-row-control {
  flex-shrink: 0;
  display: flex;
  align-items: center;
}

.km-gen-sound-control {
  gap: 6px;
}

.km-gen-preview-btn {
  width: 28px;
  height: 28px;
  display: inline-flex;
  align-items: center;
  justify-content: center;
  border-radius: var(--km-radius-md);
  border: 1px solid var(--km-border);
  background: transparent;
  color: var(--km-text-muted);
  cursor: pointer;
  padding: 0;
}

.km-gen-preview-btn:disabled {
  opacity: 0.45;
  cursor: not-allowed;
}
</style>
