<template>
  <div class="km-appearance-page">
    <!-- 分组一：明暗外观（三态并排按钮） -->
    <section class="km-app-group">
      <h4 class="km-app-group-title">明暗外观</h4>
      <div class="km-app-card">
        <div class="km-app-row">
          <div class="km-app-row-text">
            <span class="km-app-row-title">外观模式</span>
            <span class="km-app-row-desc">
              控制主窗口与设置窗口使用浅色、深色或跟随 Windows 系统设置
            </span>
          </div>
          <div class="km-app-row-control">
            <div class="km-seg-buttons" role="group" aria-label="外观模式">
              <button
                v-for="opt in DARK_MODE_OPTIONS"
                :key="opt.value"
                type="button"
                class="km-seg-btn"
                :class="{ primary: isDarkModeActive(opt.value) }"
                @click="setDarkMode(opt.value)"
              >
                {{ opt.label }}
              </button>
            </div>
          </div>
        </div>
      </div>
    </section>

    <!-- 分组二：主题配色（3 列双态迷你窗主题卡） -->
    <section class="km-app-group km-app-group-wide">
      <h4 class="km-app-group-title">主题配色</h4>
      <p class="km-app-group-desc">
        点击卡片即时切换主窗口与设置窗口的界面与按钮强调色
      </p>
      <div class="km-app-card km-theme-card">
        <div
          v-for="(row, rowIdx) in themeRows"
          :key="rowIdx"
          class="km-theme-row"
        >
          <button
            v-for="theme in row"
            :key="theme.id"
            type="button"
            class="km-theme-item"
            :class="{ selected: state.preferences.value.themeId === theme.id }"
            @click="setTheme(theme.id)"
          >
            <span class="km-theme-previews">
              <ThemeMiniWindow :themeId="theme.id" colorMode="light" />
              <ThemeMiniWindow :themeId="theme.id" colorMode="dark" />
            </span>
            <span class="km-theme-label">
              <span
                class="km-theme-label-text"
                :class="{ selected: state.preferences.value.themeId === theme.id }"
              >
                {{ theme.label }}
              </span>
              <SettingsIcon
                v-if="state.preferences.value.themeId === theme.id"
                name="check"
                :size="12"
                class="km-theme-check"
              />
            </span>
          </button>
        </div>
      </div>
    </section>

    <!-- 分组三：应用图标（当前仅内置默认图标，对齐 GPUI WP-05 icon_picker） -->
    <section class="km-app-group">
      <h4 class="km-app-group-title">应用图标</h4>
      <p class="km-app-group-desc">
        窗口与任务栏使用的应用图标。当前资源库仅内置默认图标，后续图标包发布后可在此选择
      </p>
      <div class="km-app-card">
        <div class="km-app-icon-row">
          <div class="km-app-icon-card">
            <span class="km-app-icon-tile">
              <SettingsIcon name="mail-open" :size="16" />
            </span>
            <span class="km-app-icon-label">
              <span class="km-app-icon-label-text">默认图标</span>
              <SettingsIcon name="check" :size="12" />
            </span>
          </div>
        </div>
      </div>
    </section>

    <!-- 分组四：字体缩放与窗口 chrome -->
    <section class="km-app-group">
      <h4 class="km-app-group-title">字体缩放与窗口 chrome</h4>
      <div class="km-app-card">
        <div class="km-app-row">
          <div class="km-app-row-text">
            <span class="km-app-row-title">界面字体缩放</span>
            <span class="km-app-row-desc">调整全应用控件字号比例（不影响操作系统 DPI）</span>
          </div>
          <div class="km-app-row-control">
            <ComboSelect
              :options="scaleOptions"
              :modelValue="String(state.preferences.value.uiTextScalePercent)"
              aria-label="界面字体缩放"
              @update:modelValue="setTextScale"
            />
          </div>
        </div>

        <div class="km-app-sep" />

        <div class="km-app-row">
          <div class="km-app-row-text">
            <span class="km-app-row-title">打开偏好设置时优先显示「邮箱账号」</span>
          </div>
          <div class="km-app-row-control">
            <!-- 对应字段尚未进入前端 IPC 契约，先按旧版呈现（会话内生效） -->
            <PillSwitch v-model="settingsOpenAccounts" />
          </div>
        </div>
      </div>
    </section>

    <!-- 分组五：阅读器工具栏 -->
    <section class="km-app-group">
      <h4 class="km-app-group-title">阅读器工具栏</h4>
      <p class="km-app-group-desc">
        每个按钮可选择左组（固定显示）/ 右组（窄时折叠进 ⋯）/ 不显示，并可用箭头在组内排序（完整渲染接线随后续工具栏工作落地）
      </p>
      <div class="km-app-card">
        <div class="km-app-row">
          <div class="km-app-row-text">
            <span class="km-app-row-title">始终折叠右侧工具栏辅助按钮</span>
            <span class="km-app-row-desc">
              将已读切换、原文预览、导出 EML、一键退订等收敛至更多菜单
            </span>
          </div>
          <div class="km-app-row-control">
            <!-- 对应字段尚未进入前端 IPC 契约，先按旧版呈现（会话内生效） -->
            <PillSwitch v-model="readerToolbarFoldRight" />
          </div>
        </div>

        <template v-for="item in TOOLBAR_ITEMS" :key="item.key">
          <div class="km-app-sep" />
          <div class="km-app-row">
            <div class="km-app-row-text">
              <span class="km-app-row-title">{{ item.label }}</span>
            </div>
            <div class="km-app-row-control km-app-toolbar-control">
              <button
                type="button"
                class="km-icon-ghost-btn"
                title="上移"
                aria-label="上移"
                :disabled="!isMoveEnabled(item.key, -1)"
                @click="moveItem(item.key, -1)"
              >
                <SettingsIcon name="arrow-up" :size="13" />
              </button>
              <button
                type="button"
                class="km-icon-ghost-btn"
                title="下移"
                aria-label="下移"
                :disabled="!isMoveEnabled(item.key, 1)"
                @click="moveItem(item.key, 1)"
              >
                <SettingsIcon name="arrow-down" :size="13" />
              </button>
              <ComboSelect
                class="km-app-toolbar-zone"
                :options="TOOLBAR_ZONE_OPTIONS"
                :modelValue="zoneOf(item.key)"
                :title="`${item.label} 所在分区`"
                @update:modelValue="(zone) => setZone(item.key, zone)"
              />
            </div>
          </div>
        </template>

        <div class="km-app-sep" />
        <div class="km-app-toolbar-footer">
          <button type="button" class="km-outline-btn" @click="restoreToolbar">
            恢复默认布局
          </button>
        </div>
      </div>
    </section>
  </div>
</template>

<script setup lang="ts">
import { computed, ref } from "vue";
import ComboSelect from "./ComboSelect.vue";
import PillSwitch from "./PillSwitch.vue";
import SettingsIcon from "./SettingsIcon.vue";
import ThemeMiniWindow from "./ThemeMiniWindow.vue";
import type { FullPreferencesDto } from "../preferences/types";
import type { PrefsPanelState } from "../preferences/panels/state";
import {
  TOOLBAR_ITEMS,
  TOOLBAR_ZONE_OPTIONS,
  TOOLBAR_LEFT_MAX,
  defaultToolbarLayout,
  indexInZone,
  moveWithin,
  place,
  toolbarSide,
  type ToolbarLayout,
  type ToolbarSide,
} from "./readerToolbar";

/**
 * 「外观」页（对齐 GPUI pages.rs#render_appearance + widgets.rs#theme_gallery/theme_mini_window）：
 * 三态明暗按钮、7 张双态迷你窗主题卡（每行三张）、应用图标、字号档位下拉与
 * 阅读器工具栏逐行分区控件。已进 IPC 契约的字段直连 usePreferences。
 */
const props = defineProps<{ state: PrefsPanelState }>();

const DARK_MODE_OPTIONS = [
  { label: "跟随系统", value: "system" },
  { label: "浅色", value: "light" },
  { label: "深色", value: "dark" },
] as const;

const THEME_LIST = [
  { id: "system", label: "系统" },
  { id: "default", label: "经典蓝" },
  { id: "midnight", label: "子夜" },
  { id: "tidal", label: "海潮" },
  { id: "rose", label: "玫瑰" },
  { id: "earth", label: "大地" },
  { id: "forest", label: "森林" },
] as const;

const UI_TEXT_SCALE_STEPS = [90, 100, 110, 120, 135, 150] as const;

const themeRows = computed(() => {
  const rows: Array<Array<{ id: string; label: string }>> = [];
  for (let i = 0; i < THEME_LIST.length; i += 3) {
    rows.push(THEME_LIST.slice(i, i + 3) as unknown as Array<{ id: string; label: string }>);
  }
  return rows;
});

const scaleOptions = UI_TEXT_SCALE_STEPS.map((step) => ({
  label: step === 100 ? "100%（默认）" : `${step}%`,
  value: String(step),
}));

function isDarkModeActive(option: "system" | "light" | "dark"): boolean {
  const darkMode = props.state.preferences.value.darkMode;
  if (option === "system") return darkMode === null;
  if (option === "light") return darkMode === false;
  return darkMode === true;
}

function setDarkMode(option: "system" | "light" | "dark") {
  const darkMode =
    option === "dark" ? true : option === "light" ? false : null;
  void props.state.updatePreference({ darkMode });
}

function setTheme(themeId: string) {
  void props.state.updatePreference({ themeId });
}

function setTextScale(value: string) {
  const num = Number(value);
  if (!Number.isFinite(num)) return;
  void props.state.updatePreference({ uiTextScalePercent: num });
}

// ---- 尚未进入 FullPreferencesDto 的字段（会话内生效） ----
const settingsOpenAccounts = ref(false);
const readerToolbarFoldRight = ref(false);
const toolbarLayout = ref<ToolbarLayout>(defaultToolbarLayout());

function zoneOf(key: string): string {
  return toolbarSide(toolbarLayout.value, key) ?? "hidden";
}

function setZone(key: string, zone: string) {
  const side: ToolbarSide | null =
    zone === "left" ? "left" : zone === "right" ? "right" : null;
  toolbarLayout.value = place(toolbarLayout.value, key, side);
}

function isMoveEnabled(key: string, delta: -1 | 1): boolean {
  const side = toolbarSide(toolbarLayout.value, key);
  if (!side) return false;
  const idx = indexInZone(toolbarLayout.value, key);
  const len = toolbarLayout.value[side].length;
  return delta === -1 ? idx > 0 : idx + 1 < len;
}

function moveItem(key: string, delta: -1 | 1) {
  toolbarLayout.value = moveWithin(toolbarLayout.value, key, delta);
}

function restoreToolbar() {
  toolbarLayout.value = defaultToolbarLayout();
}

// 满员提示：左组达到 7 个时隐藏项的分区切换到左组不会生效（对齐 place 钳制）。
void TOOLBAR_LEFT_MAX;
</script>

<style scoped>
.km-appearance-page {
  display: flex;
  flex-direction: column;
  align-items: center;
  gap: 20px;
  width: 100%;
}

.km-app-group {
  display: flex;
  flex-direction: column;
  gap: 6px;
  width: 100%;
  max-width: 600px;
}

.km-app-group-wide {
  max-width: 640px;
}

.km-app-group-title {
  margin: 0;
  padding: 0 4px;
  font-size: 14px;
  font-weight: 600;
  color: var(--km-text);
}

.km-app-group-desc {
  margin: 0;
  padding: 0 4px;
  font-size: 12px;
  color: var(--km-text-muted);
}

.km-app-card {
  background: var(--km-surface-raised);
  border: 1px solid var(--km-border);
  border-radius: 12px;
  overflow: hidden;
  width: 100%;
}

.km-app-sep {
  width: 100%;
  height: 1px;
  background: var(--km-border);
  flex-shrink: 0;
}

.km-app-row {
  width: 100%;
  min-height: 52px;
  padding: 10px 16px;
  gap: 12px;
  display: flex;
  align-items: center;
  justify-content: space-between;
}

.km-app-row-text {
  flex: 1;
  min-width: 0;
  display: flex;
  flex-direction: column;
  gap: 2px;
}

.km-app-row-title {
  font-size: 13px;
  font-weight: 500;
  color: var(--km-text);
  word-break: break-word;
}

.km-app-row-desc {
  font-size: 12px;
  color: var(--km-text-muted);
  word-break: break-word;
}

.km-app-row-control {
  flex-shrink: 0;
  display: flex;
  align-items: center;
}

/* 三态并排按钮（GPUI compact primary/outline） */
.km-seg-buttons {
  display: inline-flex;
  align-items: center;
  gap: 6px;
}

.km-seg-btn {
  height: 28px;
  padding: 0 12px;
  border-radius: var(--km-radius-md);
  border: 1px solid var(--km-border);
  background: transparent;
  color: var(--km-text);
  font-size: 12px;
  font-weight: 600;
  cursor: pointer;
  white-space: nowrap;
}

.km-seg-btn:hover:not(.primary) {
  background: var(--km-hover-bg);
}

.km-seg-btn.primary {
  border-color: var(--km-accent);
  background: var(--km-accent);
  color: var(--km-accent-fg);
}

/* 主题卡画廊：每行三张 */
.km-theme-card {
  padding: 12px;
  display: flex;
  flex-direction: column;
  gap: 10px;
}

.km-theme-row {
  display: flex;
  gap: 10px;
  width: 100%;
}

.km-theme-item {
  flex: 1;
  min-width: 0;
  display: flex;
  flex-direction: column;
  align-items: center;
  gap: 8px;
  padding: 10px;
  border-radius: 10px;
  border: 1px solid var(--km-border);
  background: var(--km-surface);
  cursor: pointer;
}

.km-theme-item:hover:not(.selected) {
  background: color-mix(in srgb, var(--km-surface) 75%, transparent);
}

.km-theme-item.selected {
  border-width: 2px;
  border-color: var(--km-accent);
  background: color-mix(in srgb, var(--km-accent) 10%, transparent);
  /* 2px 选中描边比普通 1px 边框宽 1px，内缩保持内容不跳 */
  padding: 9px;
}

.km-theme-previews {
  display: flex;
  align-items: center;
  gap: 6px;
}

.km-theme-label {
  display: flex;
  align-items: center;
  gap: 4px;
}

.km-theme-label-text {
  font-size: 12px;
  font-weight: 500;
  color: var(--km-text);
}

.km-theme-label-text.selected {
  font-weight: 600;
  color: var(--km-accent);
}

.km-theme-check {
  color: var(--km-accent);
}

/* 应用图标卡（当前仅默认图标） */
.km-app-icon-row {
  padding: 12px 16px;
  display: flex;
}

.km-app-icon-card {
  width: 120px;
  padding: 8px;
  gap: 6px;
  border-radius: 10px;
  background: color-mix(in srgb, var(--km-accent) 10%, transparent);
  border: 2px solid var(--km-accent);
  display: flex;
  flex-direction: column;
  align-items: center;
}

.km-app-icon-tile {
  width: 100%;
  height: 44px;
  border-radius: 6px;
  background: var(--km-accent);
  color: var(--km-accent-fg);
  display: flex;
  align-items: center;
  justify-content: center;
}

.km-app-icon-label {
  display: flex;
  align-items: center;
  gap: 4px;
  color: var(--km-accent);
}

.km-app-icon-label-text {
  font-size: 12px;
  font-weight: 500;
}

/* 阅读器工具栏行控件 */
.km-app-toolbar-control {
  gap: 4px;
}

.km-app-toolbar-zone {
  margin-left: 2px;
}

.km-icon-ghost-btn {
  width: 28px;
  height: 28px;
  display: inline-flex;
  align-items: center;
  justify-content: center;
  border: none;
  border-radius: var(--km-radius-sm);
  background: transparent;
  color: var(--km-text-muted);
  cursor: pointer;
  padding: 0;
  flex-shrink: 0;
}

.km-icon-ghost-btn:hover:not(:disabled) {
  background: var(--km-hover-bg);
  color: var(--km-text);
}

.km-icon-ghost-btn:disabled {
  opacity: 0.45;
  cursor: not-allowed;
}

.km-app-toolbar-footer {
  padding: 10px 16px;
  display: flex;
  justify-content: flex-end;
}

.km-outline-btn {
  height: 28px;
  padding: 0 12px;
  border-radius: var(--km-radius-md);
  border: 1px solid var(--km-border);
  background: transparent;
  color: var(--km-text);
  font-size: 12px;
  font-weight: 600;
  cursor: pointer;
}

.km-outline-btn:hover {
  background: var(--km-hover-bg);
}
</style>
