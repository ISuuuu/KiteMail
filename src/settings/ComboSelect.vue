<template>
  <div ref="rootRef" class="km-combo">
    <button
      type="button"
      class="km-combo-btn"
      :class="{ open }"
      :disabled="disabled"
      aria-haspopup="listbox"
      :aria-expanded="open"
      :title="title"
      @click.stop="toggle"
      @keydown.down.prevent="open = true"
      @keydown.escape.prevent="open = false"
    >
      <span class="km-combo-label">{{ currentLabel }}</span>
      <SettingsIcon name="chevron-down" :size="12" class="km-combo-chevron" />
    </button>

    <ul v-if="open" class="km-combo-menu" role="listbox">
      <li
        v-for="opt in options"
        :key="opt.value"
        class="km-combo-item"
        role="option"
        :aria-selected="opt.value === modelValue"
        @click.stop="choose(opt.value)"
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
const open = ref(false);

const currentLabel = computed(
  () => props.options.find((o) => o.value === props.modelValue)?.label ?? props.modelValue
);

function toggle() {
  if (props.disabled) return;
  open.value = !open.value;
}

function choose(value: string) {
  open.value = false;
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

.km-combo-item:hover {
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
