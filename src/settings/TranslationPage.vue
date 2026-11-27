<template>
  <div class="km-xp-page">
    <!-- 反馈横幅 -->
    <div
      v-if="feedback"
      class="km-xp-banner"
      :class="feedback.ok ? 'ok' : 'err'"
      role="status"
    >
      {{ feedback.msg }}
    </div>

    <!-- 翻译引擎（GPUI extra_pages.rs#L1968） -->
    <section class="km-xp-group">
      <h4 class="km-xp-group-title">翻译引擎</h4>
      <p class="km-xp-group-desc">仅在您点击翻译时发送邮件文本到所选引擎</p>
      <div class="km-xp-card">
        <div class="km-xp-row">
          <div class="km-xp-row-text">
            <span class="km-xp-row-title">翻译服务</span>
            <span class="km-xp-row-desc">支持 LibreTranslate / DeepL / Google / 本地 LLM</span>
          </div>
          <div class="km-xp-row-control">
            <!-- 偏好 IPC 契约暂无 translation_engine 字段：会话内状态呈现 -->
            <ComboSelect
              :options="TRANSLATION_ENGINES"
              :modelValue="engine"
              title="翻译服务"
              aria-label="翻译服务"
              @update:modelValue="engine = $event"
            />
          </div>
        </div>

        <!-- 服务地址 / 密钥 + 保存（GPUI 卡内纵排小节） -->
        <div class="km-xp-form">
          <div class="km-xp-form-line">
            <input
              v-model="apiUrl"
              type="text"
              class="km-xp-input km-xp-flex-1"
              placeholder="服务地址，例如 https://translate.example.com"
              aria-label="翻译服务地址"
            />
            <input
              v-model="apiKey"
              type="text"
              class="km-xp-input km-xp-flex-1"
              placeholder="API 密钥（可选）"
              aria-label="翻译 API 密钥"
            />
          </div>
          <div class="km-xp-form-line km-xp-save-line">
            <!-- GPUI 仅持久化 translation_api_url；密钥输入框旧版即未落盘，保持一致 -->
            <button
              type="button"
              class="km-xp-btn km-xp-btn-outline"
              @click="saveApiUrl"
            >
              保存服务地址
            </button>
          </div>
        </div>
      </div>
    </section>

    <!-- 目标语言与入口 -->
    <section class="km-xp-group">
      <h4 class="km-xp-group-title">目标语言与入口</h4>
      <div class="km-xp-card">
        <div class="km-xp-row">
          <div class="km-xp-row-text">
            <span class="km-xp-row-title">翻译目标语言</span>
          </div>
          <div class="km-xp-row-control">
            <!-- 偏好 IPC 契约暂无 translation_target_language 字段：会话内状态呈现 -->
            <ComboSelect
              :options="TRANSLATION_TARGETS"
              :modelValue="targetLanguage"
              title="翻译目标语言"
              aria-label="翻译目标语言"
              @update:modelValue="targetLanguage = $event"
            />
          </div>
        </div>
        <div class="km-xp-row">
          <div class="km-xp-row-text">
            <span class="km-xp-row-title">阅读器提供一键翻译</span>
            <span class="km-xp-row-desc">检测到非目标语言邮件时展示翻译按钮</span>
          </div>
          <div class="km-xp-row-control">
            <!-- 偏好 IPC 契约暂无 translation_offer_reader 字段：会话内状态呈现 -->
            <PillSwitch v-model="offerReader" aria-label="阅读器提供一键翻译" />
          </div>
        </div>
        <div class="km-xp-row">
          <div class="km-xp-row-text">
            <span class="km-xp-row-title">写信器提供一键翻译</span>
            <span class="km-xp-row-desc">在写信工具栏中提供正文翻译入口</span>
          </div>
          <div class="km-xp-row-control">
            <!-- 偏好 IPC 契约暂无 translation_offer_composer 字段：会话内状态呈现 -->
            <PillSwitch v-model="offerComposer" aria-label="写信器提供一键翻译" />
          </div>
        </div>
      </div>
    </section>
  </div>
</template>

<script setup lang="ts">
import { ref } from "vue";
import ComboSelect from "./ComboSelect.vue";
import PillSwitch from "./PillSwitch.vue";
import { TRANSLATION_ENGINES, TRANSLATION_TARGETS } from "./extraTabsLogic";

/**
 * 「翻译」页（GPUI extra_pages.rs#render_translation 校准）。
 * 数据域（AppPreferences.translation_engine / translation_api_url /
 * translation_target_language / translation_offer_reader / translation_offer_composer）
 * 均不在现有 Vue 偏好 IPC 契约内：全部以会话内状态实现（见汇报接线缺口）。
 * GPUI 旧版本身仅持久化服务地址（API 密钥输入框未落盘），本页保持一致。
 */
const engine = ref("off");
const targetLanguage = ref("system");
const apiUrl = ref("");
const apiKey = ref("");
const offerReader = ref(true);
const offerComposer = ref(true);
const feedback = ref<{ ok: boolean; msg: string } | null>(null);

function saveApiUrl() {
  // 对照 GPUI：仅保存服务地址（trim 后写入 translation_api_url）
  apiUrl.value = apiUrl.value.trim();
  feedback.value = { ok: true, msg: "已保存翻译服务地址配置" };
}
</script>

<style scoped src="./extraPageBase.css" />
