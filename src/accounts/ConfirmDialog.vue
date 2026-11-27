<template>
  <Teleport to="body">
    <Transition name="km-confirm-fade">
      <div
        v-if="open"
        class="km-confirm-backdrop"
        role="presentation"
        @mousedown.self="onBackdropDown"
      >
        <section
          class="km-confirm-dialog"
          role="alertdialog"
          aria-modal="true"
          aria-labelledby="km-confirm-title"
        >
          <!-- 标题行：文本 + 右上角关闭（对照 preferences/mod.rs#L1634） -->
          <header class="km-confirm-header">
            <h3 id="km-confirm-title" class="km-confirm-title">{{ title }}</h3>
            <button
              type="button"
              class="km-confirm-close"
              aria-label="关闭对话框"
              :disabled="busy"
              @click="cancel()"
            >
              <svg viewBox="0 0 16 16" width="14" height="14" aria-hidden="true">
                <path
                  d="M3.5 3.5l9 9M12.5 3.5l-9 9"
                  stroke="currentColor"
                  stroke-width="1.5"
                  stroke-linecap="round"
                  fill="none"
                />
              </svg>
            </button>
          </header>

          <!-- 提示文案区：紧凑信息框 + 说明 -->
          <div class="km-confirm-body">
            <slot>
              <p v-if="message" class="km-confirm-message">{{ message }}</p>
            </slot>
            <p v-if="description" class="km-confirm-description">{{ description }}</p>
          </div>

          <!-- 按钮排布：右对齐，取消（描边）在前、确认（主色/危险）在后 -->
          <footer class="km-confirm-actions">
            <button
              type="button"
              class="km-confirm-btn km-confirm-btn-outline"
              :disabled="busy"
              @click="cancel()"
            >
              {{ cancelLabel }}
            </button>
            <button
              type="button"
              class="km-confirm-btn"
              :class="danger ? 'km-confirm-btn-danger' : 'km-confirm-btn-primary'"
              :disabled="busy"
              @click="confirm()"
            >
              {{ confirmLabel }}
            </button>
          </footer>
        </section>
      </div>
    </Transition>
  </Teleport>
</template>

<script setup lang="ts">
import { onBeforeUnmount, watch } from "vue";

const props = withDefaults(
  defineProps<{
    open: boolean;
    title: string;
    /** 提示正文（也可用默认插槽自定义内容） */
    message?: string;
    /** 补充说明（弱化色小字） */
    description?: string;
    confirmLabel?: string;
    cancelLabel?: string;
    /** 确认按钮使用危险配色（删除类操作） */
    danger?: boolean;
    busy?: boolean;
    /** 点击遮罩是否视为取消 */
    closeOnBackdrop?: boolean;
  }>(),
  {
    message: "",
    description: "",
    confirmLabel: "确认",
    cancelLabel: "取消",
    danger: false,
    busy: false,
    closeOnBackdrop: true,
  }
);

const emit = defineEmits<{
  confirm: [];
  cancel: [];
  "update:open": [open: boolean];
}>();

function cancel() {
  if (props.busy) return;
  emit("cancel");
  emit("update:open", false);
}

function confirm() {
  if (props.busy) return;
  emit("confirm");
}

function onBackdropDown() {
  if (props.closeOnBackdrop) {
    cancel();
  }
}

function handleKeydown(ev: KeyboardEvent) {
  if (!props.open) return;
  if (ev.key === "Escape") {
    ev.preventDefault();
    ev.stopPropagation();
    cancel();
  }
}

watch(
  () => props.open,
  (open) => {
    if (typeof window === "undefined") return;
    if (open) {
      window.addEventListener("keydown", handleKeydown, true);
    } else {
      window.removeEventListener("keydown", handleKeydown, true);
    }
  },
  { immediate: true }
);

onBeforeUnmount(() => {
  if (typeof window !== "undefined") {
    window.removeEventListener("keydown", handleKeydown, true);
  }
});
</script>

<style scoped>
/* 遮罩：45% 黑，不透明遮挡（对照 GPUI bg(black().opacity(0.45)).occlude()） */
.km-confirm-backdrop {
  position: fixed;
  inset: 0;
  z-index: 1200;
  display: flex;
  align-items: center;
  justify-content: center;
  padding: 24px;
  background: rgba(0, 0, 0, 0.45);
}

/* 对话框卡片：560px、圆角 12、1px 边框、大阴影、20px 内边距、16px 组间距 */
.km-confirm-dialog {
  width: min(560px, 100%);
  display: flex;
  flex-direction: column;
  gap: 16px;
  padding: 20px;
  border-radius: var(--km-radius-lg);
  border: 1px solid var(--km-border);
  background: var(--km-surface-raised);
  box-shadow: var(--km-shadow-popover);
}

.km-confirm-header {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 8px;
}

.km-confirm-title {
  margin: 0;
  font-size: 15px;
  font-weight: 600;
  color: var(--km-text);
}

.km-confirm-close {
  width: 26px;
  height: 26px;
  display: inline-flex;
  align-items: center;
  justify-content: center;
  border: none;
  border-radius: var(--km-radius-sm);
  background: transparent;
  color: var(--km-text-muted);
  cursor: pointer;
  flex-shrink: 0;
}

.km-confirm-close:hover:not(:disabled) {
  background: var(--km-hover-bg);
  color: var(--km-text);
}

.km-confirm-body {
  display: flex;
  flex-direction: column;
  gap: 8px;
  min-width: 0;
}

.km-confirm-message {
  margin: 0;
  font-size: 13px;
  color: var(--km-text);
  overflow-wrap: anywhere;
  user-select: text;
}

.km-confirm-description {
  margin: 0;
  font-size: 12px;
  color: var(--km-text-muted);
  overflow-wrap: anywhere;
  user-select: text;
}

.km-confirm-actions {
  display: flex;
  flex-wrap: wrap;
  justify-content: flex-end;
  gap: 8px;
}

.km-confirm-btn {
  height: 30px;
  padding: 0 12px;
  border-radius: var(--km-radius-sm);
  border: 1px solid transparent;
  font-size: 12px;
  font-weight: 600;
  cursor: pointer;
}

.km-confirm-btn:disabled {
  opacity: 0.55;
  cursor: not-allowed;
}

.km-confirm-btn-outline {
  border-color: var(--km-border);
  background: transparent;
  color: var(--km-text);
}

.km-confirm-btn-outline:hover:not(:disabled) {
  background: var(--km-hover-bg);
}

.km-confirm-btn-primary {
  border-color: var(--km-accent);
  background: var(--km-accent);
  color: var(--km-accent-fg);
}

.km-confirm-btn-danger {
  border-color: var(--km-danger);
  background: var(--km-danger);
  color: #ffffff;
}

.km-confirm-fade-enter-active,
.km-confirm-fade-leave-active {
  transition: opacity 0.14s ease;
}

.km-confirm-fade-enter-from,
.km-confirm-fade-leave-to {
  opacity: 0;
}
</style>
