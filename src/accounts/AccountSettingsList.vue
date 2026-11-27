<template>
  <div class="as-list-page">
    <div class="as-page-inner">
      <!-- 页头：标题 + 说明 + 添加按钮（accounts.rs#L1688） -->
      <header v-if="showHeader" class="as-list-header">
        <div class="as-list-header-text">
          <h2 class="as-list-title">邮箱账号</h2>
          <p class="as-list-subtitle">管理已连接的邮箱账号，拖拽调整侧栏显示顺序</p>
        </div>
        <button
          v-if="!demoMode"
          type="button"
          class="as-btn as-btn-primary as-btn-compact"
          @click="emit('add')"
        >
          <svg viewBox="0 0 16 16" width="13" height="13" aria-hidden="true">
            <path
              d="M8 3.2v9.6M3.2 8h9.6"
              stroke="currentColor"
              stroke-width="1.6"
              stroke-linecap="round"
              fill="none"
            />
          </svg>
          添加账号…
        </button>
      </header>

      <!-- 演示模式提示条 -->
      <div v-if="demoMode" class="as-demo-banner">
        <p class="as-demo-title">当前处于演示模式</p>
        <p class="as-demo-desc">演示模式下使用内置模拟数据，账号配置只读。</p>
      </div>

      <!-- 状态反馈条（保存 / 测试 / 启停结果） -->
      <div
        v-if="state.statusBanner.value"
        class="as-feedback-banner"
        :class="state.statusBanner.value.kind === 'error' ? 'is-error' : 'is-success'"
        role="status"
      >
        {{ state.statusBanner.value.text }}
      </div>

      <!-- 单张分行卡片（libadwaita boxed-list） -->
      <div class="as-account-card" :class="{ highlighted }">
        <!-- 空态（accounts.rs#L1474） -->
        <div v-if="state.loading.value && state.accounts.value.length === 0" class="as-empty">
          <p class="as-empty-desc">正在加载账号列表…</p>
        </div>
        <div v-else-if="state.loadError.value" class="as-empty">
          <p class="as-empty-error" role="alert">{{ state.loadError.value }}</p>
          <button type="button" class="as-btn as-btn-outline as-btn-compact" @click="emit('retry')">
            重试
          </button>
        </div>
        <div v-else-if="state.accounts.value.length === 0" class="as-empty">
          <svg viewBox="0 0 24 24" width="32" height="32" class="as-empty-icon" aria-hidden="true">
            <circle cx="12" cy="12" r="9.2" fill="none" stroke="currentColor" stroke-width="1.6" />
            <circle cx="12" cy="10" r="3" fill="none" stroke="currentColor" stroke-width="1.6" />
            <path
              d="M6.2 18.4c1.2-2.6 3.3-4 5.8-4s4.6 1.4 5.8 4"
              fill="none"
              stroke="currentColor"
              stroke-width="1.6"
              stroke-linecap="round"
            />
          </svg>
          <p class="as-empty-title">尚未添加任何邮箱账号</p>
          <p class="as-empty-desc">支持 Microsoft 365 / Outlook OAuth2 及主流 IMAP / SMTP 服务商</p>
          <button
            v-if="!demoMode"
            type="button"
            class="as-btn as-btn-primary as-btn-compact as-empty-add"
            @click="emit('add')"
          >
            <svg viewBox="0 0 16 16" width="13" height="13" aria-hidden="true">
              <path
                d="M8 3.2v9.6M3.2 8h9.6"
                stroke="currentColor"
                stroke-width="1.6"
                stroke-linecap="round"
                fill="none"
              />
            </svg>
            添加账号…
          </button>
        </div>

        <!-- 账号行 -->
        <template v-else>
          <template v-for="(acc, idx) in state.accounts.value" :key="acc.id">
            <div
              class="as-row"
              :class="{
                'is-disabled': !acc.enabled,
                'drag-over-top':
                  reorderable && dragOverIndex === idx && dropSide === 'before',
                'drag-over-bottom':
                  reorderable && dragOverIndex === idx && dropSide === 'after',
              }"
              :draggable="reorderable"
              role="button"
              tabindex="0"
              @click="emit('select', acc.id)"
              @keydown.enter.prevent="emit('select', acc.id)"
              @dragstart="onDragStart(idx, $event)"
              @dragover="onDragOver(idx, $event)"
              @dragleave="onDragLeave(idx)"
              @drop="onDrop(idx, $event)"
              @dragend="onDragEnd"
            >
              <!-- 拖动把手 -->
              <span
                class="as-row-grip"
                :class="{ 'is-dim': demoMode }"
                :title="reorderable ? '拖拽调整顺序' : undefined"
                aria-hidden="true"
              >
                <svg viewBox="0 0 16 16" width="14" height="14">
                  <g fill="currentColor">
                    <circle cx="5.5" cy="3.5" r="1.1" />
                    <circle cx="10.5" cy="3.5" r="1.1" />
                    <circle cx="5.5" cy="8" r="1.1" />
                    <circle cx="10.5" cy="8" r="1.1" />
                    <circle cx="5.5" cy="12.5" r="1.1" />
                    <circle cx="10.5" cy="12.5" r="1.1" />
                  </g>
                </svg>
              </span>

              <!-- 品牌头像（30px 内盘 + 强调色圆环，avatar.rs#account_circle_visual） -->
              <span
                class="as-row-avatar-wrap"
                :class="{ 'is-disabled': !acc.enabled }"
                aria-hidden="true"
              >
                <span
                  class="as-avatar"
                  :style="{ borderColor: accentHex(acc) }"
                >
                  <span
                    class="as-avatar-inner"
                    :class="{ 'is-padded': brandId(acc) && !isTileBrand(brandId(acc) as string) }"
                    :style="{ background: accentHex(acc) }"
                  >
                    <img
                      v-if="brandId(acc)"
                      class="as-avatar-brand"
                      :class="{ 'is-tile': isTileBrand(brandId(acc) as string) }"
                      :src="BRAND_IMAGE_URLS[brandId(acc) as string]"
                      alt=""
                    />
                    <span v-else class="as-avatar-initials">{{ initialsOf(acc) }}</span>
                  </span>
                </span>
              </span>

              <!-- 两行文本 -->
              <span class="as-row-info" :class="{ 'is-disabled': !acc.enabled }">
                <span class="as-row-name">{{ displayLabel(acc) }}</span>
                <span v-if="secondaryText(acc)" class="as-row-secondary">
                  {{ secondaryText(acc) }}
                </span>
              </span>

              <!-- 服务商标记（品牌 PNG 或通用 IMAP 徽章，render_brand_mark） -->
              <span class="as-row-brand" aria-hidden="true">
                <img
                  v-if="brandId(acc)"
                  class="as-row-brand-img"
                  :src="BRAND_IMAGE_URLS[brandId(acc) as string]"
                  alt=""
                />
                <span v-else class="as-row-brand-pill">IMAP</span>
              </span>

              <!-- 客户端标记 -->
              <span class="as-row-app-pill" aria-hidden="true">KiteMail</span>

              <!-- 启用开关 -->
              <button
                type="button"
                role="switch"
                class="as-switch"
                :class="{ checked: acc.enabled, 'is-disabled': demoMode }"
                :aria-checked="acc.enabled"
                :aria-label="acc.enabled ? `停用 ${acc.email}` : `启用 ${acc.email}`"
                :title="acc.enabled ? '已启用同步（点击停用）' : '已停用同步（点击启用）'"
                :disabled="demoMode || state.togglingEnabledId.value === acc.id"
                @click.stop="emit('toggle', acc.id, !acc.enabled)"
              >
                <span class="as-switch-thumb" />
              </button>

              <svg
                class="as-row-chevron"
                viewBox="0 0 16 16"
                width="12"
                height="12"
                aria-hidden="true"
              >
                <path
                  d="M6 3.5L10.5 8L6 12.5"
                  fill="none"
                  stroke="currentColor"
                  stroke-width="1.6"
                  stroke-linecap="round"
                  stroke-linejoin="round"
                />
              </svg>
            </div>
            <div v-if="idx + 1 < state.accounts.value.length" class="as-row-separator" />
          </template>
        </template>
      </div>
    </div>
  </div>
</template>

<script setup lang="ts">
import { ref } from "vue";
import {
  BRAND_IMAGE_URLS,
  accountAccentHex,
  accountDisplayLabel,
  brandForAccount,
  isTileBrand,
  senderInitials,
} from "./brand";
import type { AccountConfigDto } from "./types";
import type { AccountsStore } from "./useAccounts";

const props = withDefaults(
  defineProps<{
    /** 账号域共享状态（useAccounts 返回值），由宿主创建并传入 */
    state: AccountsStore;
    demoMode?: boolean;
    /** 搜索命中等场景的卡片高亮（强调边框 + 10% 强调底） */
    highlighted?: boolean;
    /** 是否渲染页头（标题 + 添加按钮） */
    showHeader?: boolean;
    /** 是否允许拖拽排序（需宿主接线 reorder 持久化） */
    reorderable?: boolean;
  }>(),
  {
    demoMode: false,
    highlighted: false,
    showHeader: true,
    reorderable: false,
  }
);

const emit = defineEmits<{
  add: [];
  select: [accountId: string];
  toggle: [accountId: string, enabled: boolean];
  reorder: [fromIndex: number, toIndex: number];
  retry: [];
}>();

const dragIndex = ref<number | null>(null);
const dragOverIndex = ref<number | null>(null);
const dropSide = ref<"before" | "after" | null>(null);

function accentHex(acc: AccountConfigDto): string {
  return accountAccentHex(acc.email);
}

function brandId(acc: AccountConfigDto): string | null {
  return brandForAccount(acc);
}

function initialsOf(acc: AccountConfigDto): string {
  return senderInitials(displayLabel(acc) || acc.email);
}

function displayLabel(acc: AccountConfigDto): string {
  return accountDisplayLabel(acc);
}

/** 副标题：主标题即邮箱时显示昵称，否则显示邮箱（accounts.rs#L1521）。 */
function secondaryText(acc: AccountConfigDto): string {
  const label = displayLabel(acc);
  return label === acc.email ? acc.displayName.trim() : acc.email;
}

function onDragStart(idx: number, ev: DragEvent) {
  if (!props.reorderable) {
    ev.preventDefault();
    return;
  }
  dragIndex.value = idx;
  ev.dataTransfer?.setData("text/plain", String(idx));
  if (ev.dataTransfer) {
    ev.dataTransfer.effectAllowed = "move";
  }
}

function onDragOver(idx: number, ev: DragEvent) {
  if (!props.reorderable || dragIndex.value === null) return;
  ev.preventDefault();
  if (ev.dataTransfer) {
    ev.dataTransfer.dropEffect = "move";
  }
  dragOverIndex.value = idx;
  dropSide.value = dragIndex.value > idx ? "before" : dragIndex.value < idx ? "after" : null;
}

function onDragLeave(idx: number) {
  if (dragOverIndex.value === idx) {
    dragOverIndex.value = null;
    dropSide.value = null;
  }
}

function onDrop(idx: number, ev: DragEvent) {
  ev.preventDefault();
  const from = dragIndex.value;
  onDragEnd();
  if (from !== null && from !== idx) {
    emit("reorder", from, idx);
  }
}

function onDragEnd() {
  dragIndex.value = null;
  dragOverIndex.value = null;
  dropSide.value = null;
}
</script>

<style scoped>
/* 页面容器：横向 24px、纵向 20px 内边距（GPUI px_6 py_5） */
.as-list-page {
  flex: 1;
  min-height: 0;
  overflow-y: auto;
  padding: 20px 24px;
}

.as-page-inner {
  width: 100%;
  max-width: 600px;
  margin: 0 auto;
  display: flex;
  flex-direction: column;
  gap: 16px;
}

/* 页头 */
.as-list-header {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 12px;
}

.as-list-header-text {
  flex: 1;
  min-width: 0;
  display: flex;
  flex-direction: column;
  gap: 2px;
}

.as-list-title {
  margin: 0;
  font-size: 17px;
  font-weight: 700;
  color: var(--km-text);
}

.as-list-subtitle {
  margin: 0;
  font-size: 12px;
  color: var(--km-text-muted);
}

/* 演示模式条 */
.as-demo-banner {
  display: flex;
  flex-direction: column;
  gap: 4px;
  padding: 12px;
  border-radius: 10px;
  border: 1px solid color-mix(in srgb, var(--km-accent) 45%, transparent);
  background: var(--km-accent-soft);
}

.as-demo-title {
  margin: 0;
  font-size: 13px;
  font-weight: 600;
  color: var(--km-text);
}

.as-demo-desc {
  margin: 0;
  font-size: 12px;
  color: var(--km-text-muted);
}

/* 反馈条（保存 / 测试连接 / 启停结果） */
.as-feedback-banner {
  padding: 8px 12px;
  border-radius: var(--km-radius-md);
  border: 1px solid;
  font-size: 12px;
  overflow-wrap: anywhere;
}

.as-feedback-banner.is-success {
  border-color: color-mix(in srgb, var(--km-accent) 45%, transparent);
  background: var(--km-accent-soft);
  color: var(--km-accent);
}

.as-feedback-banner.is-error {
  border-color: color-mix(in srgb, var(--km-danger) 45%, transparent);
  background: color-mix(in srgb, var(--km-danger) 10%, transparent);
  color: var(--km-danger);
}

/* 单张分行卡片 */
.as-account-card {
  width: 100%;
  border-radius: var(--km-radius-lg);
  border: 1px solid var(--km-border);
  background: var(--km-surface-raised);
  overflow: hidden;
}

.as-account-card.highlighted {
  border-color: var(--km-accent);
  background: color-mix(in srgb, var(--km-accent) 10%, var(--km-surface-raised));
}

/* 空态 */
.as-empty {
  display: flex;
  flex-direction: column;
  align-items: center;
  justify-content: center;
  gap: 8px;
  padding: 40px 16px;
  text-align: center;
}

.as-empty-icon {
  color: var(--km-text-muted);
}

.as-empty-title {
  margin: 0;
  font-size: 13px;
  font-weight: 500;
  color: var(--km-text);
}

.as-empty-desc {
  margin: 0;
  font-size: 12px;
  color: var(--km-text-muted);
}

.as-empty-error {
  margin: 0;
  font-size: 12px;
  color: var(--km-danger);
}

.as-empty-add {
  margin-top: 4px;
}

/* 账号行：最小高 62px、横向 14px、纵向 10px、间距 12px */
.as-row {
  display: flex;
  align-items: center;
  gap: 12px;
  width: 100%;
  min-height: 62px;
  padding: 10px 14px;
  cursor: pointer;
  user-select: none;
}

.as-row:hover {
  background: color-mix(in srgb, var(--km-surface) 65%, transparent);
}

.as-row:focus-visible {
  outline: 2px solid var(--km-accent);
  outline-offset: -2px;
}

/* 拖拽落点指示：上方 / 下方 2px 强调线（accounts.rs#L1565） */
.as-row.drag-over-top {
  box-shadow: inset 0 2px 0 0 var(--km-accent);
}

.as-row.drag-over-bottom {
  box-shadow: inset 0 -2px 0 0 var(--km-accent);
}

.as-row-grip {
  display: inline-flex;
  flex-shrink: 0;
  color: var(--km-text-muted);
  opacity: 0.75;
}

.as-row-grip.is-dim {
  opacity: 0.3;
}

.as-row-avatar-wrap.is-disabled,
.as-row-info.is-disabled {
  opacity: 0.55;
}

.as-row-info.is-disabled {
  opacity: 0.6;
}

/* 头像：外环 2px 强调色 + 1px 留白 + 30px 内盘 */
.as-avatar {
  display: flex;
  align-items: center;
  justify-content: center;
  width: 36px;
  height: 36px;
  border-radius: 999px;
  border: 2px solid;
  padding: 1px;
  flex-shrink: 0;
}

.as-avatar-inner {
  display: flex;
  align-items: center;
  justify-content: center;
  width: 30px;
  height: 30px;
  border-radius: 999px;
  overflow: hidden;
}

.as-avatar-brand {
  width: 100%;
  height: 100%;
  object-fit: contain;
}

.as-avatar-brand.is-tile {
  object-fit: cover;
}

/* 非满铺品牌：白底 + 少量内边距（GPUI pad size*0.14） */
.as-avatar-inner.is-padded {
  background: #ffffff;
  padding: 4px;
}

.as-avatar-initials {
  color: #ffffff;
  font-size: 12.5px;
  font-weight: 700;
  line-height: 1;
}

/* 两行文本 */
.as-row-info {
  flex: 1;
  min-width: 0;
  display: flex;
  flex-direction: column;
  gap: 2px;
}

.as-row-name {
  font-size: 13px;
  font-weight: 500;
  color: var(--km-text);
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.as-row-secondary {
  font-size: 12px;
  color: var(--km-text-muted);
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}

/* 服务商标记列（宽 44px 居中） */
.as-row-brand {
  width: 44px;
  display: flex;
  align-items: center;
  justify-content: center;
  flex-shrink: 0;
}

.as-row-brand-img {
  width: 24px;
  height: 24px;
  object-fit: contain;
}

.as-row-brand-pill {
  width: 38px;
  height: 22px;
  border-radius: var(--km-radius-sm);
  background: #005bff;
  color: #ffffff;
  font-size: 10px;
  font-weight: 700;
  display: inline-flex;
  align-items: center;
  justify-content: center;
  letter-spacing: 0.02em;
}

/* KiteMail 标记药丸 */
.as-row-app-pill {
  padding: 2px 8px;
  border-radius: var(--km-radius-sm);
  border: 1px solid var(--km-border);
  background: var(--km-surface);
  color: var(--km-text-muted);
  font-size: 12px;
  flex-shrink: 0;
}

/* 行分隔线 */
.as-row-separator {
  width: 100%;
  height: 1px;
  background: var(--km-border);
}

/* 药丸开关（36×20，thumb 16，inset 2） */
.as-switch {
  position: relative;
  width: 36px;
  height: 20px;
  border-radius: 999px;
  border: none;
  background: color-mix(in srgb, var(--km-text-muted) 35%, transparent);
  padding: 2px;
  cursor: pointer;
  flex-shrink: 0;
  transition: background-color 0.15s ease;
}

.as-switch.checked {
  background: var(--km-accent);
}

.as-switch.is-disabled {
  opacity: 0.45;
  cursor: not-allowed;
}

.as-switch:disabled {
  cursor: not-allowed;
}

.as-switch-thumb {
  display: block;
  width: 16px;
  height: 16px;
  border-radius: 999px;
  background: #ffffff;
  box-shadow: 0 1px 2px rgba(0, 0, 0, 0.2);
  transition: transform 0.15s ease;
}

.as-switch.checked .as-switch-thumb {
  transform: translateX(16px);
}

.as-row-chevron {
  color: var(--km-text-muted);
  flex-shrink: 0;
}

/* 紧凑按钮（GPUI compact Medium：高 32px、内边距 8px） */
.as-btn {
  display: inline-flex;
  align-items: center;
  justify-content: center;
  gap: 6px;
  height: 32px;
  border-radius: var(--km-radius-sm);
  border: 1px solid transparent;
  padding: 0 12px;
  font-size: 12px;
  font-weight: 600;
  cursor: pointer;
  white-space: nowrap;
}

.as-btn:disabled {
  opacity: 0.55;
  cursor: not-allowed;
}

.as-btn-compact {
  padding: 0 10px;
}

.as-btn-primary {
  border-color: var(--km-accent);
  background: var(--km-accent);
  color: var(--km-accent-fg);
}

.as-btn-primary:hover:not(:disabled) {
  background: color-mix(in srgb, var(--km-accent) 88%, #00000000);
}

.as-btn-outline {
  border-color: var(--km-border);
  background: transparent;
  color: var(--km-text);
}

.as-btn-outline:hover:not(:disabled) {
  background: var(--km-hover-bg);
}
</style>
