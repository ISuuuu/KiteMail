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

    <!-- 自动收集（GPUI extra_pages.rs#L737） -->
    <section class="km-xp-group">
      <h4 class="km-xp-group-title">自动收集</h4>
      <div class="km-xp-card">
        <div class="km-xp-row" id="sp-contacts-auto-collect">
          <div class="km-xp-row-text">
            <span class="km-xp-row-title">自动收集发件人与收件人</span>
            <span class="km-xp-row-desc">
              从已同步与已发送邮件中自动聚合联系人到自动补全列表
            </span>
          </div>
          <div class="km-xp-row-control">
            <!-- 偏好 IPC 契约暂无 contacts_auto_collect_senders 字段：会话内状态呈现 -->
            <PillSwitch v-model="autoCollect" aria-label="自动收集发件人与收件人" />
          </div>
        </div>
      </div>
    </section>

    <!-- 远程通讯录源（CardDAV）列表 + 添加表单 -->
    <section class="km-xp-group">
      <h4 class="km-xp-group-title">远程通讯录源（CardDAV）</h4>
      <div class="km-xp-card" id="sp-contacts-sources">
        <div v-if="sources.length === 0" class="km-xp-empty">
          尚未添加外部 CardDAV 通讯录源。
        </div>
        <div v-for="src in sources" :key="src.id" class="km-xp-row">
          <div class="km-xp-row-text">
            <span class="km-xp-row-title">{{ src.name }}</span>
            <span class="km-xp-row-desc">
              {{ serviceEntrySubtitle(src.provider, src.serverUrl, src.username) }}
            </span>
          </div>
          <div class="km-xp-row-control">
            <PillSwitch
              :modelValue="src.enabled"
              :aria-label="`启用通讯录源 ${src.name}`"
              @update:modelValue="toggleSource(src.id, $event)"
            />
            <button
              type="button"
              class="km-xp-btn km-xp-btn-ghost"
              title="删除通讯录源"
              aria-label="删除通讯录源"
              @click="removeSource(src.id)"
            >
              <SettingsIcon name="trash" :size="14" />
            </button>
          </div>
        </div>

        <!-- 添加表单（GPUI 两行结构） -->
        <div class="km-xp-form">
          <div class="km-xp-form-line">
            <ComboSelect
              :options="CONTACT_PROVIDERS"
              :modelValue="form.provider"
              title="通讯录服务商"
              aria-label="通讯录服务商"
              @update:modelValue="form.provider = $event"
            />
            <input
              v-model="form.name"
              type="text"
              class="km-xp-input km-xp-flex-1"
              placeholder="通讯录名称"
              aria-label="通讯录源名称"
            />
          </div>
          <div class="km-xp-form-line">
            <input
              v-model="form.serverUrl"
              type="text"
              class="km-xp-input km-xp-flex-1"
              placeholder="CardDAV 地址，例如 https://dav.example.com/contacts"
              aria-label="CardDAV 地址"
            />
            <input
              v-model="form.username"
              type="text"
              class="km-xp-input km-xp-flex-1"
              placeholder="用户名"
              aria-label="通讯录源用户名"
            />
            <button
              type="button"
              class="km-xp-btn km-xp-btn-primary"
              @click="addSource"
            >
              <SettingsIcon name="plus" :size="13" />
              添加源
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
  CONTACT_PROVIDERS,
  serviceEntrySubtitle,
  type ContactSourceItem,
} from "./extraTabsLogic";

/**
 * 「通讯录源」页（GPUI extra_pages.rs#render_contacts_sources 校准）。
 * 数据域（AppPreferences.contacts_auto_collect_senders / contact_sources）不在现有
 * Vue 偏好 IPC 契约内：自动收集开关与源列表 / 新增 / 启停 / 删除以会话内状态实现
 * （见汇报接线缺口）。
 */
const sources = ref<ContactSourceItem[]>([]);
const autoCollect = ref(true);
const feedback = ref<{ ok: boolean; msg: string } | null>(null);

const form = reactive({
  provider: "CardDAV",
  name: "",
  serverUrl: "",
  username: "",
});

function addSource() {
  const name = form.name.trim();
  const serverUrl = form.serverUrl.trim();
  if (!name || !serverUrl) {
    feedback.value = { ok: false, msg: "请填写通讯录名称与 CardDAV 地址" };
    return;
  }
  const nextId = sources.value.reduce((max, s) => Math.max(max, s.id), 0) + 1;
  sources.value = [
    ...sources.value,
    {
      id: nextId,
      provider: form.provider,
      name,
      serverUrl,
      username: form.username.trim(),
      enabled: true,
    },
  ];
  form.name = "";
  form.serverUrl = "";
  form.username = "";
  feedback.value = { ok: true, msg: "已保存 CardDAV 通讯录源" };
}

function toggleSource(id: number, enabled: boolean) {
  sources.value = sources.value.map((s) => (s.id === id ? { ...s, enabled } : s));
}

function removeSource(id: number) {
  sources.value = sources.value.filter((s) => s.id !== id);
}
</script>

<style scoped src="./extraPageBase.css" />
