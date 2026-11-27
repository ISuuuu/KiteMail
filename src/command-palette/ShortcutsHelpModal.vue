<template>
  <div
    v-if="open"
    class="km-shortcuts-backdrop"
    role="dialog"
    aria-modal="true"
    aria-labelledby="km-shortcuts-title"
    @click.self="closeModal"
  >
    <section class="km-shortcuts-modal">
      <header class="km-modal-header">
        <div class="km-modal-title-group">
          <h2 id="km-shortcuts-title" class="km-modal-title">键盘快捷键速查</h2>
          <span class="km-modal-subtitle">主窗口单键操作与组合快捷键</span>
        </div>

        <button
          type="button"
          class="km-close-btn"
          title="关闭 (Esc)"
          aria-label="关闭快捷键帮助"
          @click="closeModal"
        >
          ✕
        </button>
      </header>

      <div class="km-shortcuts-body">
        <div class="km-shortcuts-note">
          提示：标注「单键」的快捷键需在「偏好设置 → 外观与布局」中启用单键快捷键；在输入框聚焦、中文输入法 (IME) 组合输入或弹窗打开时会自动让位。
        </div>

        <div class="km-groups-grid">
          <section
            v-for="group in SHORTCUT_HELP_GROUPS"
            :key="group.title"
            class="km-shortcut-group"
          >
            <h3 class="km-group-title">{{ group.title }}</h3>
            <ul class="km-shortcut-list">
              <li
                v-for="entry in group.entries"
                :key="entry.keys.join('+')"
                class="km-shortcut-row"
              >
                <span class="km-shortcut-desc">
                  {{ entry.description }}
                  <span v-if="entry.requiresSingleKey" class="km-single-tag">单键</span>
                </span>
                <span class="km-keys">
                  <kbd v-for="k in entry.keys" :key="k" class="km-kbd">{{ k }}</kbd>
                </span>
              </li>
            </ul>
          </section>
        </div>
      </div>
    </section>
  </div>
</template>

<script setup lang="ts">
import { onMounted, onUnmounted, watch } from "vue";
import { SHORTCUT_HELP_GROUPS } from "./useCommandPalette";

const props = defineProps<{
  open: boolean;
}>();

const emit = defineEmits<{
  "update:open": [open: boolean];
  close: [];
  overlayChange: [id: string, open: boolean];
}>();

const OVERLAY_ID = "shortcuts-help";

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
  }
});

onUnmounted(() => {
  detachGlobalListeners();
  if (props.open) {
    emit("overlayChange", OVERLAY_ID, false);
  }
});
</script>

<style scoped>
.km-shortcuts-backdrop {
  position: fixed;
  inset: 0;
  z-index: 1050;
  display: flex;
  align-items: center;
  justify-content: center;
  padding: 16px;
  background: rgba(0, 0, 0, 0.48);
  backdrop-filter: blur(4px);
}

.km-shortcuts-modal {
  width: min(760px, 100%);
  max-height: min(620px, calc(100vh - 32px));
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

.km-modal-subtitle {
  font-size: 12px;
  color: var(--km-text-muted);
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

.km-shortcuts-body {
  flex: 1;
  min-height: 0;
  overflow-y: auto;
  padding: 16px 20px;
  display: flex;
  flex-direction: column;
  gap: 14px;
}

.km-shortcuts-note {
  padding: 9px 12px;
  border-radius: var(--km-radius-sm);
  border: 1px solid color-mix(in srgb, var(--km-accent) 28%, var(--km-border));
  background: var(--km-accent-soft);
  font-size: 12px;
  color: var(--km-text);
}

.km-groups-grid {
  display: grid;
  grid-template-columns: repeat(2, minmax(0, 1fr));
  gap: 12px;
}

.km-shortcut-group {
  padding: 12px 14px;
  border-radius: var(--km-radius-md);
  border: 1px solid var(--km-border);
  background: var(--km-surface);
  display: flex;
  flex-direction: column;
  gap: 8px;
}

.km-group-title {
  margin: 0;
  font-size: 12px;
  font-weight: 700;
  color: var(--km-text-muted);
}

.km-shortcut-list {
  list-style: none;
  margin: 0;
  padding: 0;
  display: flex;
  flex-direction: column;
  gap: 6px;
}

.km-shortcut-row {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 10px;
  font-size: 12px;
}

.km-shortcut-desc {
  color: var(--km-text);
  display: inline-flex;
  align-items: center;
  gap: 6px;
}

.km-single-tag {
  padding: 0 5px;
  border-radius: 4px;
  background: var(--km-hover-bg);
  color: var(--km-text-muted);
  font-size: 10px;
}

.km-keys {
  display: inline-flex;
  align-items: center;
  gap: 4px;
  flex-shrink: 0;
}

.km-kbd {
  padding: 2px 6px;
  border-radius: 4px;
  border: 1px solid var(--km-border);
  background: var(--km-canvas);
  color: var(--km-text);
  font-family: var(--km-font-mono);
  font-size: 11px;
}

@media (max-width: 680px) {
  .km-groups-grid {
    grid-template-columns: 1fr;
  }
}
</style>
