<template>
  <section class="km-tab-panel">
    <h3 class="km-tab-title">关于 KiteMail 与检查更新</h3>

    <div class="km-card">
      <div class="km-about-hero">
        <div>
          <div class="km-about-name">KiteMail Desktop</div>
          <div class="km-about-meta">
            当前版本：<strong>v{{ state.appVersion.value }}</strong>
            · 运行形态：
            <span class="km-status-pill">
              {{ state.isPortable.value ? "便携版 (Portable)" : "标准安装版" }}
            </span>
          </div>
        </div>
        <button
          type="button"
          class="km-btn km-btn-primary"
          :disabled="state.checkingUpdate.value"
          @click="state.checkForUpdates()"
        >
          {{ state.checkingUpdate.value ? "检查中…" : "检查更新" }}
        </button>
      </div>

      <div v-if="state.updateResult.value" class="km-update-box">
        <div class="km-update-msg">{{ state.updateResult.value.message }}</div>
        <div class="km-update-meta">
          目标平台：{{ state.updateResult.value.target }}
          <template v-if="state.updateResult.value.latestVersion">
            · 最新版本：v{{ state.updateResult.value.latestVersion }}
          </template>
          <template v-if="state.updateResult.value.pubDate">
            · 发布日期：{{ state.updateResult.value.pubDate }}
          </template>
        </div>
        <p v-if="state.updateResult.value.notes" class="km-update-notes">
          {{ state.updateResult.value.notes }}
        </p>
        <button
          v-if="state.updateResult.value.updateAvailable"
          type="button"
          class="km-btn km-btn-primary"
          :disabled="state.installingUpdate.value"
          @click="state.installUpdate()"
        >
          {{ state.installingUpdate.value ? "安装中…" : "下载并安装更新" }}
        </button>
        <a
          v-if="state.updateResult.value.downloadUrl"
          :href="state.updateResult.value.downloadUrl"
          target="_blank"
          rel="noopener noreferrer"
          class="km-update-link"
        >
          前往下载新版本
        </a>
      </div>
    </div>
  </section>
</template>

<script setup lang="ts">
import type { PrefsPanelState } from "./state";

defineProps<{ state: PrefsPanelState }>();
</script>
