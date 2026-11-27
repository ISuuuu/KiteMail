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

    <!-- 智能文件夹（GPUI extra_pages.rs#render_smart_folders） -->
    <section class="km-xp-group" id="sp-smart-folders-page">
      <h4 class="km-xp-group-title">智能文件夹（动态虚拟视图）</h4>
      <p class="km-xp-group-desc">支持搜索语法如 from:alice is:unread has:attachment</p>
      <div class="km-xp-card">
        <div v-if="folders.length === 0" class="km-xp-empty">
          尚未创建智能文件夹。
        </div>
        <div v-for="folder in folders" :key="folder.id" class="km-xp-row">
          <div class="km-xp-row-text">
            <span class="km-xp-row-title">{{ folder.name }}</span>
            <span class="km-xp-row-desc">
              {{ smartFolderSubtitle(folder.accountEmail, folder.query) }}
            </span>
          </div>
          <div class="km-xp-row-actions">
            <button
              type="button"
              class="km-xp-btn km-xp-btn-ghost"
              title="删除智能文件夹"
              aria-label="删除智能文件夹"
              @click="removeFolder(folder.id)"
            >
              <SettingsIcon name="trash" :size="14" />
            </button>
          </div>
        </div>

        <!-- 创建表单（GPUI 两行结构） -->
        <div class="km-xp-form">
          <div class="km-xp-form-line">
            <input
              v-model="form.name"
              type="text"
              class="km-xp-input km-xp-flex-1"
              placeholder="文件夹名称"
              aria-label="文件夹名称"
            />
            <ComboSelect
              :options="SMART_ICON_OPTIONS"
              :modelValue="form.icon"
              title="图标"
              aria-label="文件夹图标"
              @update:modelValue="form.icon = $event"
            />
            <ComboSelect
              :options="accountOptions"
              :modelValue="form.accountEmail"
              title="账号范围"
              aria-label="账号范围"
              @update:modelValue="form.accountEmail = $event"
            />
          </div>
          <div class="km-xp-form-line">
            <input
              v-model="form.query"
              type="text"
              class="km-xp-input km-xp-flex-1"
              placeholder="搜索表达式，例如 from:alice is:unread"
              aria-label="搜索表达式"
              @keydown.enter.prevent="addFolder"
            />
            <button
              type="button"
              class="km-xp-btn km-xp-btn-primary"
              @click="addFolder"
            >
              <SettingsIcon name="plus" :size="13" />
              创建智能文件夹
            </button>
          </div>
        </div>
      </div>
    </section>
  </div>
</template>

<script setup lang="ts">
import { onMounted, reactive, ref } from "vue";
import ComboSelect from "./ComboSelect.vue";
import SettingsIcon from "./SettingsIcon.vue";
import {
  SMART_ICON_OPTIONS,
  smartFolderSubtitle,
  type SmartFolderItem,
} from "./extraTabsLogic";
import { listAccounts } from "../accounts/ipc";

/**
 * 「智能文件夹」页（GPUI extra_pages.rs#render_smart_folders 校准）。
 * 数据域（AppPreferences.smart_folders）不在现有 Vue 偏好 IPC 契约内：
 * 列表 / 创建 / 删除以会话内状态实现（见汇报接线缺口）。
 */
const folders = ref<SmartFolderItem[]>([]);
const feedback = ref<{ ok: boolean; msg: string } | null>(null);

const form = reactive({
  name: "",
  icon: "folder-search",
  accountEmail: "",
  query: "",
});

const accountOptions = ref([{ label: "全部账号", value: "" }]);

onMounted(() => {
  void listAccounts()
    .then((res) => {
      accountOptions.value = [
        { label: "全部账号", value: "" },
        ...res.accounts.map((a) => ({ label: a.email, value: a.email })),
      ];
    })
    .catch(() => {
      /* 账号列表加载失败时仅保留「全部账号」选项 */
    });
});

function addFolder() {
  const name = form.name.trim();
  const query = form.query.trim();
  if (!name || !query) {
    feedback.value = { ok: false, msg: "请填写名称与搜索表达式" };
    return;
  }
  const nextId = folders.value.reduce((max, f) => Math.max(max, f.id), 0) + 1;
  folders.value = [
    ...folders.value,
    {
      id: nextId,
      name,
      icon: form.icon,
      query,
      accountEmail: form.accountEmail,
    },
  ];
  form.name = "";
  form.query = "";
  feedback.value = { ok: true, msg: "已创建智能文件夹" };
}

function removeFolder(id: number) {
  folders.value = folders.value.filter((f) => f.id !== id);
}
</script>

<style scoped src="./extraPageBase.css" />
