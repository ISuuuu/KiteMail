<template>
  <div
    v-if="open"
    class="km-palette-backdrop"
    role="dialog"
    aria-modal="true"
    aria-label="命令面板"
    @click.self="closeModal"
  >
    <section class="km-palette-modal">
      <div class="km-palette-input-wrap">
        <input
          ref="inputRef"
          :value="palette.query.value"
          type="text"
          class="km-palette-input"
          placeholder="输入命令名称、分类或快捷键… (Esc 关闭)"
          aria-label="搜索命令"
          @input="onQueryInput"
          @keydown="onInputKeydown"
        />
        <kbd class="km-palette-hint">Ctrl+K</kbd>
      </div>

      <div v-if="palette.filteredItems.value.length === 0" class="km-palette-empty">
        未找到匹配「{{ palette.query.value }}」的命令
      </div>

      <ul v-else class="km-palette-list" role="listbox" aria-label="命令列表">
        <li
          v-for="(item, idx) in palette.filteredItems.value"
          :key="item.id"
          role="option"
          :aria-selected="idx === palette.activeIndex.value"
          class="km-palette-item"
          :class="{
            active: idx === palette.activeIndex.value,
            disabled: Boolean(item.disabled),
          }"
          @mouseenter="!item.disabled && (palette.activeIndex.value = idx)"
          @click="executeItem(item)"
        >
          <div class="km-palette-item-main">
            <span class="km-palette-category">{{ item.category }}</span>
            <span class="km-palette-title">{{ item.title }}</span>
          </div>
          <kbd v-if="item.shortcut" class="km-palette-kbd">{{ item.shortcut }}</kbd>
        </li>
      </ul>
    </section>
  </div>
</template>

<script setup lang="ts">
import { nextTick, onMounted, onUnmounted, ref, watch } from "vue";
import type { CommandPaletteItem } from "./types";
import { isImeComposing, useCommandPalette } from "./useCommandPalette";

const props = defineProps<{
  open: boolean;
  items: CommandPaletteItem[];
}>();

const emit = defineEmits<{
  "update:open": [open: boolean];
  close: [];
  execute: [commandId: string];
  overlayChange: [id: string, open: boolean];
}>();

const OVERLAY_ID = "command-palette";
const inputRef = ref<HTMLInputElement | null>(null);

const palette = useCommandPalette(() => props.items);

function onQueryInput(ev: Event) {
  palette.setQuery((ev.target as HTMLInputElement).value);
}

function executeItem(item: CommandPaletteItem) {
  if (item.disabled) return;
  emit("execute", item.id);
  closeModal();
}

function onInputKeydown(ev: KeyboardEvent) {
  if (isImeComposing(ev)) return;
  if (ev.key === "ArrowDown") {
    ev.preventDefault();
    palette.moveSelection(1);
    return;
  }
  if (ev.key === "ArrowUp") {
    ev.preventDefault();
    palette.moveSelection(-1);
    return;
  }
  if (ev.key === "Enter") {
    ev.preventDefault();
    const active = palette.getActiveItem();
    if (active) executeItem(active);
    return;
  }
  if (ev.key === "Escape") {
    ev.preventDefault();
    ev.stopPropagation();
    closeModal();
  }
}

function closeModal() {
  if (!props.open) return;
  emit("overlayChange", OVERLAY_ID, false);
  emit("update:open", false);
  emit("close");
}

function handleGlobalKeydown(ev: KeyboardEvent) {
  if (!props.open || isImeComposing(ev)) return;
  if (ev.key === "Escape") {
    ev.preventDefault();
    ev.stopPropagation();
    closeModal();
  }
}

function attachGlobalListeners() {
  if (typeof window !== "undefined") {
    window.addEventListener("keydown", handleGlobalKeydown);
  }
}

function detachGlobalListeners() {
  if (typeof window !== "undefined") {
    window.removeEventListener("keydown", handleGlobalKeydown);
  }
}

watch(
  () => props.open,
  (isOpen, prevOpen) => {
    if (isOpen && !prevOpen) {
      palette.reset();
      emit("overlayChange", OVERLAY_ID, true);
      attachGlobalListeners();
      void nextTick(() => inputRef.value?.focus());
    } else if (!isOpen && prevOpen) {
      detachGlobalListeners();
      emit("overlayChange", OVERLAY_ID, false);
    }
  }
);

onMounted(() => {
  if (props.open) {
    palette.reset();
    emit("overlayChange", OVERLAY_ID, true);
    attachGlobalListeners();
    void nextTick(() => inputRef.value?.focus());
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
.km-palette-backdrop {
  position: fixed;
  inset: 0;
  z-index: 1100;
  display: flex;
  align-items: flex-start;
  justify-content: center;
  padding: 72px 16px 24px;
  background: rgba(0, 0, 0, 0.42);
  backdrop-filter: blur(3px);
}

.km-palette-modal {
  width: min(600px, 100%);
  max-height: min(460px, calc(100vh - 108px));
  display: flex;
  flex-direction: column;
  border-radius: var(--km-radius-lg);
  border: 1px solid var(--km-border);
  background: var(--km-surface-raised);
  box-shadow: var(--km-shadow-popover);
  overflow: hidden;
}

.km-palette-input-wrap {
  display: flex;
  align-items: center;
  gap: 8px;
  padding: 10px 14px;
  border-bottom: 1px solid var(--km-border);
  background: var(--km-surface);
}

.km-palette-input {
  flex: 1;
  min-width: 0;
  border: none;
  background: transparent;
  color: var(--km-text);
  font-size: 14px;
  outline: none;
  user-select: text;
}

.km-palette-hint,
.km-palette-kbd {
  padding: 2px 6px;
  border-radius: 4px;
  border: 1px solid var(--km-border);
  background: var(--km-canvas);
  color: var(--km-text-muted);
  font-family: var(--km-font-mono);
  font-size: 11px;
  flex-shrink: 0;
}

.km-palette-empty {
  padding: 28px 16px;
  text-align: center;
  color: var(--km-text-muted);
  font-size: 13px;
}

.km-palette-list {
  list-style: none;
  margin: 0;
  padding: 6px;
  overflow-y: auto;
  display: flex;
  flex-direction: column;
  gap: 2px;
}

.km-palette-item {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 10px;
  padding: 8px 10px;
  border-radius: var(--km-radius-sm);
  cursor: pointer;
}

.km-palette-item:hover:not(.disabled),
.km-palette-item.active:not(.disabled) {
  background: var(--km-accent-soft);
}

.km-palette-item.active:not(.disabled) .km-palette-title {
  color: var(--km-accent);
  font-weight: 600;
}

.km-palette-item.disabled {
  opacity: 0.45;
  cursor: not-allowed;
}

.km-palette-item-main {
  display: flex;
  align-items: center;
  gap: 8px;
  min-width: 0;
}

.km-palette-category {
  padding: 1px 6px;
  border-radius: 4px;
  background: var(--km-hover-bg);
  color: var(--km-text-muted);
  font-size: 11px;
  flex-shrink: 0;
}

.km-palette-title {
  font-size: 13px;
  color: var(--km-text);
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}
</style>
