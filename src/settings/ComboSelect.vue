<template>
  <div ref="rootRef" class="km-combo">
    <button
      ref="buttonRef"
      type="button"
      class="km-combo-btn"
      :class="{ open }"
      :disabled="disabled"
      aria-haspopup="listbox"
      :aria-expanded="open"
      :aria-activedescendant="open ? menuId : undefined"
      :title="title"
      @click.stop="toggle"
      @keydown.down.prevent="onArrowDown"
      @keydown.up.prevent="onArrowUp"
      @keydown.enter.prevent="onEnter"
      @keydown.escape.prevent="open = false"
      @keydown.tab="open = false"
      @keydown.home.prevent="open && (highlighted = 0)"
      @keydown.end.prevent="open && (highlighted = options.length - 1)"
    >
      <span class="km-combo-label">{{ currentLabel }}</span>
      <SettingsIcon name="chevron-down" :size="12" class="km-combo-chevron" />
    </button>

    <ul
      v-if="open"
      :id="menuId"
      class="km-combo-menu"
      role="listbox"
    >
      <li
        v-for="(opt, idx) in options"
        :id="`${menuId}-opt-${idx}`"
        :key="opt.value"
        class="km-combo-item"
        role="option"
        :class="{ highlighted: idx === highlighted }"
        :aria-selected="opt.value === modelValue"
        @click.stop="choose(opt.value)"
        @mousemove="highlighted = idx"
      >
        <span class="km-combo-check">
          <SettingsIcon
            v-if="opt.value === modelValue"
            name="check"
            :size="12"
          />
        </span>
        <span class="km-combo-item-label">{{ opt.label }}</span>
      </li>
    </ul>
  </div>
</template>

<script setup lang="ts">
import { computed, onBeforeUnmount, onMounted, ref } from "vue";
import SettingsIcon from "./SettingsIcon.vue";

/**
 * 静态选项下拉（对齐 GPUI widgets.rs#combo_button）：
 * outline 紧凑按钮 + 弹出菜单，当前项前显示勾选标记。
 * 键盘导航（WP-10）：焦点保持在按钮上（aria-activedescendant），
 * ↓/↑ 循环移动高亮、Enter 确认高亮项、Esc 关闭、Home/End 跳首尾、Tab 关闭。
 */
export interface ComboOption {
  label: string;
  value: string;
}

const props = defineProps<{
  options: readonly ComboOption[];
  modelValue: string;
  disabled?: boolean;
  title?: string;
}>();

const emit = defineEmits<{ "update:modelValue": [value: string] }>();

const rootRef = ref<HTMLElement | null>(null);
const buttonRef = ref<HTMLButtonElement | null>(null);
const open = ref(false);
const highlighted = ref(0);

// 同页多个 ComboSelect 需要互不冲突的菜单 / 选项元素 id（aria-activedescendant）
const menuId = `km-combo-menu-${Math.random().toString(36).slice(2, 9)}`;

const currentLabel = computed(
  () => props.options.find((o) => o.value === props.modelValue)?.label ?? props.modelValue
);

function openMenu() {
  if (props.disabled || open.value) return;
  const selectedIndex = props.options.findIndex((o) => o.value === props.modelValue);
  highlighted.value = selectedIndex >= 0 ? selectedIndex : 0;
  open.value = true;
}

function toggle() {
  if (props.disabled) return;
  if (open.value) {
    open.value = false;
  } else {
    openMenu();
  }
}

function moveHighlight(delta: number) {
  if (!open.value) {
    openMenu();
    return;
  }
  const count = props.options.length;
  if (count === 0) return;
  // 循环移动（对齐 GPUI 菜单上下键循环选择）
  highlighted.value = (highlighted.value + delta + count) % count;
}

function onArrowDown() {
  moveHighlight(1);
}

function onArrowUp() {
  moveHighlight(-1);
}

function onEnter() {
  if (!open.value) {
    openMenu();
    return;
  }
  const opt = props.options[highlighted.value];
  if (opt) {
    choose(opt.value);
  }
}

function choose(value: string) {
  open.value = false;
  buttonRef.value?.focus();
  if (value !== props.modelValue) {
    emit("update:modelValue", value);
  }
}

function onDocPointerDown(event: PointerEvent) {
  if (open.value && rootRef.value && !rootRef.value.contains(event.target as Node)) {
    open.value = false;
  }
}

onMounted(() => {
  document.addEventListener("pointerdown", onDocPointerDown, true);
});

onBeforeUnmount(() => {
  document.removeEventListener("pointerdown", onDocPointerDown, true);
});
</script>

<style scoped>
.km-combo {
  position: relative;
  display: inline-flex;
  flex-shrink: 0;
}

.km-combo-btn {
  height: 30px;
  padding: 0 10px;
  gap: 6px;
  border-radius: var(--km-radius-md);
  border: 1px solid var(--km-border);
  background: transparent;
  color: var(--km-text);
  font-size: 12px;
  font-weight: 600;
  cursor: pointer;
  display: inline-flex;
  align-items: center;
  max-width: 220px;
}

.km-combo-btn:hover:not(:disabled),
.km-combo-btn.open {
  background: var(--km-hover-bg);
}

.km-combo-btn:disabled {
  opacity: 0.45;
  cursor: not-allowed;
}

.km-combo-label {
  white-space: nowrap;
  overflow: hidden;
  text-overflow: ellipsis;
}

.km-combo-chevron {
  flex-shrink: 0;
  color: var(--km-text-muted);
}

.km-combo-menu {
  position: absolute;
  top: calc(100% + 4px);
  right: 0;
  z-index: 60;
  min-width: 100%;
  margin: 0;
  padding: 4px;
  list-style: none;
  border-radius: var(--km-radius-md);
  border: 1px solid var(--km-border);
  background: var(--km-surface-raised);
  box-shadow: var(--km-shadow-popover);
  max-height: 264px;
  overflow-y: auto;
}

.km-combo-item {
  display: flex;
  align-items: center;
  gap: 6px;
  padding: 6px 8px;
  border-radius: var(--km-radius-sm);
  font-size: 12px;
  color: var(--km-text);
  cursor: pointer;
  white-space: nowrap;
}

.km-combo-item:hover,
.km-combo-item.highlighted {
  background: var(--km-hover-bg);
}

.km-combo-check {
  width: 12px;
  height: 12px;
  flex-shrink: 0;
  display: inline-flex;
  align-items: center;
  justify-content: center;
  color: var(--km-accent);
}

.km-combo-item[aria-selected="true"] .km-combo-item-label {
  font-weight: 600;
}
</style>
