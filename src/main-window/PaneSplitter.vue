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

/* 将分隔线视觉/布局占宽保持为 1 px，并通过伪元素在边界两侧保留 7 px 指针命中区 */
.km-splitter::before {
  content: "";
  position: absolute;
  z-index: 5;
}

.km-splitter-vertical {
  width: 1px;
  height: 100%;
  cursor: col-resize;
}

.km-splitter-vertical::before {
  top: 0;
  bottom: 0;
  left: -3px;
  right: -3px;
}

.km-splitter-horizontal {
  height: 1px;
  width: 100%;
  cursor: row-resize;
}

.km-splitter-horizontal::before {
  left: 0;
  right: 0;
  top: -3px;
  bottom: -3px;
}

.km-splitter-line {
  position: relative;
  background: var(--km-border);
  transition: background-color 120ms ease;
  pointer-events: none;
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

/* 悬停/拖拽高亮加粗浮于边界之上，不挤占相邻栏布局宽度 */
.km-splitter-vertical:hover .km-splitter-line::after,
.km-splitter-vertical.dragging .km-splitter-line::after,
.km-splitter-vertical:focus-visible .km-splitter-line::after {
  content: "";
  position: absolute;
  top: 0;
  bottom: 0;
  left: -1px;
  width: 3px;
  background: var(--km-accent);
}

.km-splitter-horizontal:hover .km-splitter-line::after,
.km-splitter-horizontal.dragging .km-splitter-line::after,
.km-splitter-horizontal:focus-visible .km-splitter-line::after {
  content: "";
  position: absolute;
  left: 0;
  right: 0;
  top: -1px;
  height: 3px;
  background: var(--km-accent);
}
</style>
