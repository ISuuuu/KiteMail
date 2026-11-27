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

    <!-- 立即运行（GPUI extra_pages.rs#L1656） -->
    <section class="km-xp-group" id="sp-filters-page">
      <h4 class="km-xp-group-title">立即运行</h4>
      <p class="km-xp-group-desc">
        对本地 SQLite 缓存的现有邮件执行已启用的过滤规则（标为已读 / 标星 / 移动或归档）
      </p>
      <div class="km-xp-card">
        <div class="km-xp-row">
          <div class="km-xp-row-text">
            <span class="km-xp-row-title">在现有邮件上运行过滤器</span>
            <span class="km-xp-row-desc">仅处理已开启的规则</span>
          </div>
          <div class="km-xp-row-control">
            <!-- 后端过滤执行 IPC 缺失：按旧版禁用态呈现（GPUI demo_mode / 空规则禁用） -->
            <button
              type="button"
              class="km-xp-btn km-xp-btn-outline"
              disabled
              title="等待过滤执行后端接口接入"
            >
              <SettingsIcon name="play" :size="13" />
              立即在现有邮件上运行
            </button>
          </div>
        </div>
      </div>
    </section>

    <!-- 自动化规则列表 + 新增表单 -->
    <section class="km-xp-group">
      <h4 class="km-xp-group-title">自动化规则列表</h4>
      <div class="km-xp-card">
        <div v-if="rules.length === 0" class="km-xp-empty">
          尚未创建自动化过滤规则。
        </div>
        <div v-for="rule in rules" :key="rule.id" class="km-xp-row">
          <div class="km-xp-row-text">
            <span class="km-xp-row-title">
              {{ rule.name.trim() || `规则 #${rule.id}` }}
            </span>
            <span class="km-xp-row-desc">{{ buildFilterRuleSummary(rule) }}</span>
          </div>
          <div class="km-xp-row-control">
            <PillSwitch
              :modelValue="rule.enabled"
              :aria-label="`启用规则 ${rule.name.trim() || rule.id}`"
              @update:modelValue="toggleRule(rule.id, $event)"
            />
            <button
              type="button"
              class="km-xp-btn km-xp-btn-ghost"
              title="删除规则"
              aria-label="删除规则"
              @click="removeRule(rule.id)"
            >
              <SettingsIcon name="trash" :size="14" />
            </button>
          </div>
        </div>

        <!-- 新增表单（GPUI 三行结构） -->
        <div class="km-xp-form">
          <div class="km-xp-form-line">
            <input
              v-model="form.name"
              type="text"
              class="km-xp-input km-xp-flex-1"
              placeholder="规则名称（可选）"
              aria-label="规则名称"
            />
            <ComboSelect
              :options="accountOptions"
              :modelValue="form.accountEmail"
              title="生效账号范围"
              aria-label="生效账号"
              @update:modelValue="form.accountEmail = $event"
            />
          </div>
          <div class="km-xp-form-line">
            <ComboSelect
              :options="FILTER_FIELD_OPTIONS"
              :modelValue="form.field"
              title="过滤字段"
              aria-label="过滤字段"
              @update:modelValue="form.field = $event as FilterField"
            />
            <ComboSelect
              :options="FILTER_MATCH_OPTIONS"
              :modelValue="form.matcher"
              title="匹配方式"
              aria-label="匹配方式"
              @update:modelValue="form.matcher = $event as FilterMatch"
            />
            <input
              v-model="form.value"
              type="text"
              class="km-xp-input km-xp-flex-1"
              placeholder="匹配关键词"
              aria-label="匹配关键词"
            />
          </div>
          <div class="km-xp-form-line">
            <input
              v-model="form.moveToFolder"
              type="text"
              class="km-xp-input km-xp-flex-1"
              placeholder="移动到文件夹（可选）"
              aria-label="移动到文件夹"
            />
            <input
              v-model="form.applyTag"
              type="text"
              class="km-xp-input km-xp-flex-1"
              placeholder="打标签名称（可选）"
              aria-label="打标签名称"
            />
            <input
              v-model="form.forwardTo"
              type="text"
              class="km-xp-input km-xp-flex-1"
              placeholder="转发到邮箱（可选）"
              aria-label="转发到邮箱"
            />
          </div>
          <div class="km-xp-form-line">
            <div class="km-xp-row-actions">
              <button
                type="button"
                class="km-xp-btn km-xp-btn-outline"
                :class="{ selected: form.archive }"
                @click="form.archive = !form.archive"
              >
                归档
              </button>
              <button
                type="button"
                class="km-xp-btn km-xp-btn-outline"
                :class="{ selected: form.star }"
                @click="form.star = !form.star"
              >
                标星
              </button>
              <button
                type="button"
                class="km-xp-btn km-xp-btn-outline"
                :class="{ selected: form.markRead }"
                @click="form.markRead = !form.markRead"
              >
                标为已读
              </button>
            </div>
            <span class="km-xp-flex-1" />
            <button
              type="button"
              class="km-xp-btn km-xp-btn-primary"
              @click="addRule"
            >
              <SettingsIcon name="plus" :size="13" />
              添加规则
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
import PillSwitch from "./PillSwitch.vue";
import SettingsIcon from "./SettingsIcon.vue";
import {
  FILTER_FIELD_OPTIONS,
  FILTER_MATCH_OPTIONS,
  buildFilterRuleSummary,
  type FilterField,
  type FilterMatch,
  type FilterRuleItem,
} from "./extraTabsLogic";
import { listAccounts } from "../accounts/ipc";
import type { AccountConfigDto } from "../accounts/types";

/**
 * 「过滤器」页（GPUI extra_pages.rs#render_filters 校准）。
 * 规则数据域（AppPreferences.filter_rules）不在现有 Vue 偏好 IPC 契约内：
 * 规则列表 / 新增 / 启停 / 移除以会话内状态实现；
 * 「立即在现有邮件上运行」需后端过滤执行接口，维持禁用态（见汇报接线缺口）。
 */
const rules = ref<FilterRuleItem[]>([]);
const feedback = ref<{ ok: boolean; msg: string } | null>(null);
const accounts = ref<AccountConfigDto[]>([]);

const form = reactive({
  name: "",
  accountEmail: "",
  field: "from" as FilterField,
  matcher: "contains" as FilterMatch,
  value: "",
  moveToFolder: "",
  applyTag: "",
  forwardTo: "",
  archive: false,
  star: false,
  markRead: false,
});

const accountOptions = ref([
  { label: "全部账号", value: "" },
]);

onMounted(() => {
  // 生效账号范围下拉来自账号 IPC（settings 窗已放行 allow-list-accounts）
  void listAccounts()
    .then((res) => {
      accounts.value = res.accounts;
      accountOptions.value = [
        { label: "全部账号", value: "" },
        ...res.accounts.map((a) => ({ label: a.email, value: a.email })),
      ];
    })
    .catch(() => {
      /* 账号列表加载失败时仅保留「全部账号」选项 */
    });
});

function addRule() {
  const value = form.value.trim();
  if (!value) {
    feedback.value = { ok: false, msg: "请填写过滤匹配关键词" };
    return;
  }
  const nextId = rules.value.reduce((max, r) => Math.max(max, r.id), 0) + 1;
  rules.value = [
    ...rules.value,
    {
      id: nextId,
      name: form.name.trim(),
      accountEmail: form.accountEmail,
      enabled: true,
      field: form.field,
      matcher: form.matcher,
      value,
      moveToFolder: form.moveToFolder.trim(),
      archive: form.archive,
      star: form.star,
      markRead: form.markRead,
      applyTag: form.applyTag.trim(),
      forwardTo: form.forwardTo.trim(),
    },
  ];
  form.name = "";
  form.value = "";
  form.moveToFolder = "";
  form.applyTag = "";
  form.forwardTo = "";
  form.archive = false;
  form.star = false;
  form.markRead = false;
  feedback.value = { ok: true, msg: "已添加过滤器规则" };
}

function toggleRule(id: number, enabled: boolean) {
  rules.value = rules.value.map((r) => (r.id === id ? { ...r, enabled } : r));
}

function removeRule(id: number) {
  rules.value = rules.value.filter((r) => r.id !== id);
}
</script>

<style scoped src="./extraPageBase.css" />
