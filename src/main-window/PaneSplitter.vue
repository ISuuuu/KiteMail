<template>
  <div
    class="km-splitter"
    :class="[`km-splitter-${direction}`, { dragging }]"
    role="separator"
    :aria-orientation="direction === 'vertical' ? 'vertical' : 'horizontal'"
    :aria-label="label ?? (direction === 'vertical' ? '调整栏宽' : '调整栏高')"
    :aria-valuenow="value"
    :aria-valuemin="min"
    :aria-valuemax="max"
    tabindex="0"
    @pointerdown="onPointerDown"
    @pointermove="onPointerMove"
    @pointerup="onPointerUp"
    @pointercancel="onPointerUp"
    @keydown="onKeyDown"
  >
    <div class="km-splitter-line" />
  </div>
</template>

<script setup lang="ts">
import { ref } from "vue";

const props = defineProps<{
  direction: "vertical" | "horizontal";
  value?: number;
  min?: number;
  max?: number;
  label?: string;
}>();

const emit = defineEmits<{
  dragStart: [];
  drag: [totalDelta: number];
  dragEnd: [];
  step: [delta: number];
}>();

const dragging = ref(false);
let startCoord = 0;
let activePointerId: number | null = null;

function onPointerDown(e: PointerEvent) {
  if (e.button !== 0) return;
  e.preventDefault();
  dragging.value = true;
  activePointerId = e.pointerId;
  startCoord = props.direction === "vertical" ? e.clientX : e.clientY;
  (e.currentTarget as HTMLElement | null)?.setPointerCapture(e.pointerId);
  emit("dragStart");
}

function onPointerMove(e: PointerEvent) {
  if (!dragging.value || e.pointerId !== activePointerId) return;
  const current = props.direction === "vertical" ? e.clientX : e.clientY;
  emit("drag", current - startCoord);
}

function onPointerUp(e: PointerEvent) {
  if (!dragging.value || e.pointerId !== activePointerId) return;
  dragging.value = false;
  activePointerId = null;
  if ((e.currentTarget as HTMLElement | null)?.hasPointerCapture(e.pointerId)) {
    (e.currentTarget as HTMLElement | null)?.releasePointerCapture(e.pointerId);
  }
  emit("dragEnd");
}

function onKeyDown(e: KeyboardEvent) {
  if (e.isComposing || e.key === "Process" || e.keyCode === 229) return;
  const stepSize = e.shiftKey ? 40 : 12;
  if (props.direction === "vertical") {
    if (e.key === "ArrowLeft") {
      e.preventDefault();
      emit("step", -stepSize);
    } else if (e.key === "ArrowRight") {
      e.preventDefault();
      emit("step", stepSize);
    }
  } else {
    if (e.key === "ArrowUp") {
      e.preventDefault();
      emit("step", -stepSize);
    } else if (e.key === "ArrowDown") {
      e.preventDefault();
      emit("step", stepSize);
    }
  }
}
</script>

<style scoped>
.km-splitter {
  position: relative;
  flex-shrink: 0;
  display: flex;
  align-items: center;
  justify-content: center;
  background: transparent;
  z-index: 5;
  touch-action: none;
}

.km-splitter-vertical {
  width: 5px;
  height: 100%;
  cursor: col-resize;
}

.km-splitter-horizontal {
  height: 5px;
  width: 100%;
  cursor: row-resize;
}

.km-splitter-line {
  background: var(--km-border);
  transition: background-color 120ms ease;
}

.km-splitter-vertical .km-splitter-line {
  width: 1px;
  height: 100%;
}

.km-splitter-horizontal .km-splitter-line {
  height: 1px;
  width: 100%;
}

.km-splitter:hover .km-splitter-line,
.km-splitter.dragging .km-splitter-line,
.km-splitter:focus-visible .km-splitter-line {
  background: var(--km-accent);
}

.km-splitter-vertical:hover .km-splitter-line,
.km-splitter-vertical.dragging .km-splitter-line {
  width: 2px;
}

.km-splitter-horizontal:hover .km-splitter-line,
.km-splitter-horizontal.dragging .km-splitter-line {
  height: 2px;
}
</style>
