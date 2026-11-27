<template>
  <div class="km-xp-page">
    <!-- 顶部实时搜索（GPUI pages.rs#render_senders 搜索栏在卡外） -->
    <div class="km-xp-search-bar">
      <SettingsIcon name="search" :size="15" class="km-xp-search-icon" />
      <input
        v-model="filterText"
        type="search"
        class="km-xp-input km-xp-flex-1"
        placeholder="搜索已屏蔽的发件人或域名"
        aria-label="搜索已屏蔽的发件人或域名"
      />
      <button
        v-if="filterText.trim()"
        type="button"
        class="km-xp-btn km-xp-btn-ghost"
        @click="filterText = ''"
      >
        清除筛选
      </button>
    </div>

    <!-- 远图策略提示（block_remote_images 关闭时展示，对照 pages.rs#L454） -->
    <div v-if="!blockRemoteImages" class="km-xp-remote-hint">
      <span>提示：当前已关闭「默认屏蔽远程图像」，所有发件人的外链图片均会直接加载。</span>
      <button
        type="button"
        class="km-xp-btn km-xp-btn-outline"
        @click="emit('gotoPrivacy')"
      >
        前往隐私设置
      </button>
    </div>

    <!-- 操作反馈 -->
    <div
      v-if="feedback"
      class="km-xp-banner km-xp-banner-wide"
      :class="feedback.ok ? 'ok' : 'err'"
      role="status"
    >
      {{ feedback.msg }}
    </div>

    <!-- 已屏蔽发件人黑名单 -->
    <section class="km-xp-group km-xp-group-wide">
      <h4 class="km-xp-group-title">已屏蔽发件人黑名单（{{ blocked.length }}）</h4>
      <p class="km-xp-group-desc">支持填写完整邮箱（user@example.com）或域名（example.com）</p>
      <div class="km-xp-card">
        <!-- 添加栏 -->
        <div class="km-xp-form">
          <div class="km-xp-form-line">
            <input
              v-model="newEntry"
              type="text"
              class="km-xp-input km-xp-flex-1"
              placeholder="邮箱地址或域名，例如 user@example.com / example.com"
              aria-label="要屏蔽的邮箱地址或域名"
              @keydown.enter.prevent="addBlocked"
            />
            <button
              type="button"
              class="km-xp-btn km-xp-btn-primary"
              @click="addBlocked"
            >
              添加到黑名单
            </button>
          </div>
        </div>

        <!-- 两种空态：黑名单为空 / 筛选无结果 -->
        <div v-if="filtered.length === 0" class="km-xp-empty-center">
          <SettingsIcon name="ban" :size="15" />
          <span>
            {{
              blocked.length === 0
                ? "黑名单暂无发件人或域名；添加后将拦截来自该发件人或域名的邮件。"
                : "没有匹配当前搜索词的已屏蔽发件人。"
            }}
          </span>
        </div>

        <!-- 黑名单行 -->
        <div
          v-for="entry in filtered"
          :key="entry"
          class="km-xp-row"
        >
          <div class="km-xp-row-text">
            <span class="km-xp-row-title">{{ entry }}</span>
          </div>
          <div class="km-xp-row-actions">
            <button
              type="button"
              class="km-xp-btn km-xp-btn-ghost"
              @click="removeBlocked(entry)"
            >
              <SettingsIcon name="trash" :size="14" />
              移除
            </button>
          </div>
        </div>
      </div>
    </section>
  </div>
</template>

<script setup lang="ts">
import { computed, ref } from "vue";
import SettingsIcon from "./SettingsIcon.vue";
import {
  addToBlocklist,
  filterBlocklist,
  removeFromBlocklist,
} from "./extraTabsLogic";
import type { usePreferences } from "../preferences/usePreferences";

/**
 * 「发件人」页（GPUI pages.rs#render_senders 校准）。
 * - 远图策略读取真实偏好（FullPreferencesDto.blockRemoteImages），「前往隐私设置」跳转隐私页签；
 * - 黑名单数据域（AppPreferences.blocked_senders）不在现有 Vue 偏好 IPC 契约内：
 *   新增 / 移除 / 实时筛选以会话内状态实现（见汇报接线缺口）。
 */
const props = defineProps<{
  /** 偏好共享状态（usePreferences 返回值），用于远图策略提示 */
  state: ReturnType<typeof usePreferences>;
}>();

const emit = defineEmits<{ gotoPrivacy: [] }>();

const blocked = ref<string[]>([]);
const newEntry = ref("");
const filterText = ref("");
const feedback = ref<{ ok: boolean; msg: string } | null>(null);

const blockRemoteImages = computed(() => props.state.preferences.value.blockRemoteImages);

const filtered = computed(() => filterBlocklist(blocked.value, filterText.value));

function addBlocked() {
  const raw = newEntry.value;
  if (!raw.trim()) {
    feedback.value = { ok: false, msg: "请输入邮箱地址或域名（如 example.com）" };
    return;
  }
  const result = addToBlocklist(blocked.value, raw);
  if (result.added === null) {
    feedback.value = {
      ok: false,
      msg: "请输入合法的邮箱地址或域名（如 example.com）",
    };
    return;
  }
  blocked.value = result.blocked;
  newEntry.value = "";
  feedback.value = { ok: true, msg: `已屏蔽：${result.added}` };
}

function removeBlocked(entry: string) {
  blocked.value = removeFromBlocklist(blocked.value, entry);
}
</script>

<style scoped src="./extraPageBase.css" />
