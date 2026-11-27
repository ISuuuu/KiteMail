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

    <!-- 超大附件自动转存（GPUI extra_pages.rs#L468） -->
    <section class="km-xp-group">
      <h4 class="km-xp-group-title">超大附件自动转存</h4>
      <p class="km-xp-group-desc">写信附件体积超过阈值时提示上传到云盘并插入分享链接</p>
      <div class="km-xp-card">
        <div class="km-xp-row">
          <div class="km-xp-row-text">
            <span class="km-xp-row-title">自动上传阈值</span>
            <span class="km-xp-row-desc">支持 Nextcloud / WebDAV / S3 / Dropbox</span>
          </div>
          <div class="km-xp-row-control">
            <!-- 偏好 IPC 契约暂无 cloud_auto_upload_threshold_mb 字段：会话内状态呈现 -->
            <ComboSelect
              :options="CLOUD_THRESHOLD_OPTIONS"
              :modelValue="thresholdMb"
              title="自动上传阈值"
              aria-label="自动上传阈值"
              @update:modelValue="thresholdMb = $event"
            />
          </div>
        </div>
      </div>
    </section>

    <!-- 云存储账号列表 + 添加表单 -->
    <section class="km-xp-group">
      <h4 class="km-xp-group-title">云存储账号</h4>
      <div class="km-xp-card">
        <div v-if="accounts.length === 0" class="km-xp-empty">
          尚未添加云存储账号。
        </div>
        <div v-for="acc in accounts" :key="acc.id" class="km-xp-row">
          <div class="km-xp-row-text">
            <span class="km-xp-row-title">{{ acc.name }}</span>
            <span class="km-xp-row-desc">
              {{ serviceEntrySubtitle(acc.provider, acc.serverUrl, acc.username) }}
            </span>
          </div>
          <div class="km-xp-row-control">
            <PillSwitch
              :modelValue="acc.enabled"
              :aria-label="`启用云存储账号 ${acc.name}`"
              @update:modelValue="toggleAccount(acc.id, $event)"
            />
            <button
              type="button"
              class="km-xp-btn km-xp-btn-ghost"
              title="删除云存储账号"
              aria-label="删除云存储账号"
              @click="removeAccount(acc.id)"
            >
              <SettingsIcon name="trash" :size="14" />
            </button>
          </div>
        </div>

        <!-- 添加表单（GPUI 三行结构） -->
        <div class="km-xp-form">
          <div class="km-xp-form-line">
            <ComboSelect
              :options="CLOUD_PROVIDERS"
              :modelValue="form.provider"
              title="云盘服务商"
              aria-label="云盘服务商"
              @update:modelValue="form.provider = $event"
            />
            <input
              v-model="form.name"
              type="text"
              class="km-xp-input km-xp-flex-1"
              placeholder="账号名称"
              aria-label="云存储账号名称"
            />
          </div>
          <div class="km-xp-form-line">
            <input
              v-model="form.serverUrl"
              type="text"
              class="km-xp-input km-xp-flex-1"
              placeholder="服务器地址，例如 https://cloud.example.com"
              aria-label="云存储服务器地址"
            />
            <input
              v-model="form.username"
              type="text"
              class="km-xp-input km-xp-flex-1"
              placeholder="用户名"
              aria-label="云存储用户名"
            />
          </div>
          <div class="km-xp-form-line">
            <input
              v-model="form.bucketOrPath"
              type="text"
              class="km-xp-input km-xp-flex-1"
              placeholder="存储桶 / 路径（可选）"
              aria-label="存储桶或路径"
            />
            <button
              type="button"
              class="km-xp-btn km-xp-btn-primary"
              @click="addAccount"
            >
              <SettingsIcon name="plus" :size="13" />
              添加云盘
            </button>
          </div>
        </div>
      </div>
    </section>
  </div>
</template>

<script setup lang="ts">
import { reactive, ref } from "vue";
import ComboSelect from "./ComboSelect.vue";
import PillSwitch from "./PillSwitch.vue";
import SettingsIcon from "./SettingsIcon.vue";
import {
  CLOUD_PROVIDERS,
  CLOUD_THRESHOLD_OPTIONS,
  serviceEntrySubtitle,
  type CloudStorageItem,
} from "./extraTabsLogic";

/**
 * 「云存储」页（GPUI extra_pages.rs#render_cloud_storage 校准）。
 * 数据域（AppPreferences.cloud_auto_upload_threshold_mb / cloud_accounts）不在现有
 * Vue 偏好 IPC 契约内：阈值下拉与账号列表 / 新增 / 启停 / 删除以会话内状态实现
 * （见汇报接线缺口）。
 */
const accounts = ref<CloudStorageItem[]>([]);
const thresholdMb = ref("25");
const feedback = ref<{ ok: boolean; msg: string } | null>(null);

const form = reactive({
  provider: "Nextcloud",
  name: "",
  serverUrl: "",
  username: "",
  bucketOrPath: "",
});

function addAccount() {
  const name = form.name.trim();
  const serverUrl = form.serverUrl.trim();
  if (!name || !serverUrl) {
    feedback.value = { ok: false, msg: "请填写账号名称与服务器地址" };
    return;
  }
  const nextId = accounts.value.reduce((max, a) => Math.max(max, a.id), 0) + 1;
  accounts.value = [
    ...accounts.value,
    {
      id: nextId,
      provider: form.provider,
      name,
      serverUrl,
      username: form.username.trim(),
      bucketOrPath: form.bucketOrPath.trim(),
      enabled: true,
    },
  ];
  form.name = "";
  form.serverUrl = "";
  form.username = "";
  form.bucketOrPath = "";
  feedback.value = { ok: true, msg: "已添加云存储账号配置" };
}

function toggleAccount(id: number, enabled: boolean) {
  accounts.value = accounts.value.map((a) => (a.id === id ? { ...a, enabled } : a));
}

function removeAccount(id: number) {
  accounts.value = accounts.value.filter((a) => a.id !== id);
}
</script>

<style scoped src="./extraPageBase.css" />
