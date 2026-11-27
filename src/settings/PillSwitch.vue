<template>
  <button
    type="button"
    role="switch"
    class="km-pill-switch"
    :class="{ on: modelValue, disabled }"
    :aria-checked="modelValue"
    :disabled="disabled"
    @click="toggle"
  >
    <span class="km-pill-knob" />
  </button>
</template>

<script setup lang="ts">
/** 药丸开关（对齐 GPUI Switch：选中用强调底，禁用 45% 透明度）。 */
const props = defineProps<{ modelValue: boolean; disabled?: boolean }>();
const emit = defineEmits<{ "update:modelValue": [value: boolean] }>();

function toggle() {
  if (props.disabled) return;
  emit("update:modelValue", !props.modelValue);
}
</script>

<style scoped>
.km-pill-switch {
  width: 34px;
  height: 20px;
  border-radius: 999px;
  border: 1px solid var(--km-border);
  background: var(--km-hover-bg);
  padding: 1px;
  cursor: pointer;
  display: inline-flex;
  align-items: center;
  flex-shrink: 0;
  transition: background 0.15s ease, border-color 0.15s ease;
}

.km-pill-knob {
  width: 16px;
  height: 16px;
  border-radius: 999px;
  background: var(--km-surface-raised);
  box-shadow: 0 1px 2px rgba(0, 0, 0, 0.2);
  transition: transform 0.15s ease;
}

.km-pill-switch.on {
  background: var(--km-accent);
  border-color: var(--km-accent);
}

.km-pill-switch.on .km-pill-knob {
  transform: translateX(14px);
  background: #ffffff;
}

.km-pill-switch.disabled,
.km-pill-switch:disabled {
  opacity: 0.45;
  cursor: not-allowed;
}
</style>
