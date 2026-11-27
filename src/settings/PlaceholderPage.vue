<template>
  <div class="km-set-placeholder">
    <SettingsIcon name="info" :size="20" />
    <p class="km-set-placeholder-text">
      「{{ label }}」页面内容将在后续任务中按旧版逐页迁移。
    </p>
  </div>
</template>

<script setup lang="ts">
import SettingsIcon from "./SettingsIcon.vue";

/** 尚未迁移页签的占位内容（本轮仅还原壳层与「常规」页）。 */
defineProps<{ label: string }>();
</script>

<style scoped>
.km-set-placeholder {
  display: flex;
  flex-direction: column;
  align-items: center;
  justify-content: center;
  gap: 10px;
  padding: 48px 24px;
  color: var(--km-text-muted);
  text-align: center;
}

.km-set-placeholder-text {
  margin: 0;
  font-size: 13px;
  max-width: 360px;
}
</style>
