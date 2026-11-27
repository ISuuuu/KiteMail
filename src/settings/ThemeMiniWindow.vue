<template>
  <span
    class="km-mini-win"
    :data-theme="themeId"
    :data-color-mode="colorMode"
    aria-hidden="true"
  >
    <!-- 11px 顶栏：右侧三枚窗口圆点 -->
    <span class="km-mini-titlebar">
      <span class="km-mini-dot dim" />
      <span class="km-mini-dot dim" />
      <span class="km-mini-dot" />
    </span>
    <!-- 主体：22px 侧栏 + 内容卡 -->
    <span class="km-mini-body">
      <span class="km-mini-sidebar">
        <span class="km-mini-nav-active" />
        <span class="km-mini-line w14" />
        <span class="km-mini-line w16" />
      </span>
      <span class="km-mini-content">
        <span class="km-mini-content-lines">
          <span class="km-mini-line w22 strong" />
          <span class="km-mini-line w16 muted" />
        </span>
        <span class="km-mini-btn" />
      </span>
    </span>
  </span>
</template>

<script setup lang="ts">
/**
 * 70×50 双态窗口缩略图（严格复刻 GPUI widgets.rs#theme_mini_window，
 * 即 Linux 版 theme_picker：11px 顶栏 + 22px 侧栏 + 内容卡 + 强调色按钮）。
 * 配色通过 data-theme/data-color-mode 命中 theme.css 的子树作用域变量块。
 */
defineProps<{
  themeId: string;
  colorMode: "light" | "dark";
}>();
</script>

<style scoped>
.km-mini-win {
  width: 70px;
  height: 50px;
  border-radius: 6px;
  border: 1px solid var(--km-border);
  background: var(--km-canvas);
  overflow: hidden;
  display: flex;
  flex-direction: column;
  flex-shrink: 0;
}

.km-mini-titlebar {
  height: 11px;
  padding: 0 5px;
  background: var(--km-sidebar);
  border-bottom: 1px solid var(--km-border);
  display: flex;
  align-items: center;
  justify-content: flex-end;
  gap: 2.5px;
  flex-shrink: 0;
}

.km-mini-dot {
  width: 3px;
  height: 3px;
  border-radius: 999px;
  background: color-mix(in srgb, var(--km-text-muted) 65%, transparent);
}

.km-mini-dot.dim {
  background: color-mix(in srgb, var(--km-text-muted) 45%, transparent);
}

.km-mini-body {
  flex: 1;
  min-height: 0;
  padding: 4px;
  gap: 4px;
  display: flex;
}

.km-mini-sidebar {
  width: 22px;
  height: 100%;
  display: flex;
  flex-direction: column;
  gap: 3px;
  flex-shrink: 0;
}

.km-mini-nav-active {
  width: 100%;
  height: 7px;
  border-radius: 3px;
  background: color-mix(in srgb, var(--km-accent) 12%, transparent);
}

.km-mini-line {
  height: 3px;
  border-radius: 1.5px;
  background: color-mix(in srgb, var(--km-text-muted) 25%, transparent);
  display: block;
}

.km-mini-line.w14 {
  width: 14px;
  background: color-mix(in srgb, var(--km-text-muted) 35%, transparent);
}

.km-mini-line.w16 {
  width: 16px;
}

.km-mini-line.w22 {
  width: 22px;
}

.km-mini-line.strong {
  background: color-mix(in srgb, var(--km-text) 70%, transparent);
}

.km-mini-line.muted {
  width: 16px;
  background: color-mix(in srgb, var(--km-text-muted) 40%, transparent);
}

.km-mini-content {
  flex: 1;
  height: 100%;
  border-radius: 4px;
  background: var(--km-surface-raised);
  border: 1px solid var(--km-border);
  padding: 4px;
  display: flex;
  flex-direction: column;
  justify-content: space-between;
}

.km-mini-content-lines {
  display: flex;
  flex-direction: column;
  gap: 2.5px;
}

.km-mini-btn {
  width: 12px;
  height: 5px;
  border-radius: 2.5px;
  background: var(--km-accent);
  align-self: flex-end;
  display: block;
}
</style>
