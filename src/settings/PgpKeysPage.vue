<template>
  <div class="km-xp-page">
    <!-- 登记反馈（GPUI extra_pages.rs#L418） -->
    <div
      v-if="feedback"
      class="km-xp-banner"
      :class="feedback.ok ? 'ok' : 'err'"
      role="status"
    >
      {{ feedback.msg }}
    </div>

    <!-- 密钥查询策略（偏好字段 openpgp_wkd_lookup / openpgp_keyserver_lookup） -->
    <section class="km-xp-group">
      <h4 class="km-xp-group-title">密钥查询策略</h4>
      <p class="km-xp-group-desc">
        发信时自动查找收件人公钥（本地元信息与策略持久化；Windows 端尚未内置 GnuPG/Sequoia
        加解密引擎）
      </p>
      <div class="km-xp-card">
        <div class="km-xp-row">
          <div class="km-xp-row-text">
            <span class="km-xp-row-title">通过 WKD 查询收件人公钥</span>
            <span class="km-xp-row-desc">向收件人域名发起 Web Key Directory 查询</span>
          </div>
          <div class="km-xp-row-control">
            <!-- 偏好 IPC 契约暂无 openpgp_wkd_lookup 字段：会话内状态呈现 -->
            <PillSwitch v-model="wkdLookup" aria-label="通过 WKD 查询收件人公钥" />
          </div>
        </div>
        <div class="km-xp-row">
          <div class="km-xp-row-text">
            <span class="km-xp-row-title">同时查询 keys.openpgp.org</span>
            <span class="km-xp-row-desc">从公共 OpenPGP 密钥服务器拉取公钥</span>
          </div>
          <div class="km-xp-row-control">
            <!-- 偏好 IPC 契约暂无 openpgp_keyserver_lookup 字段：会话内状态呈现 -->
            <PillSwitch v-model="keyserverLookup" aria-label="同时查询 keys.openpgp.org" />
          </div>
        </div>
      </div>
    </section>

    <!-- 已登记密钥列表 -->
    <section class="km-xp-group">
      <h4 class="km-xp-group-title">已登记密钥列表</h4>
      <p class="km-xp-group-desc">按邮箱绑定默认签名/加密策略；删除仅移除本地登记</p>
      <div class="km-xp-card">
        <div v-if="keys.length === 0" class="km-xp-empty">
          尚未登记 OpenPGP 密钥。
        </div>
        <div v-for="key in keys" :key="key.id" class="km-xp-row">
          <div class="km-xp-row-text">
            <span class="km-xp-row-title">{{ pgpKeyTitle(key) }}</span>
            <span class="km-xp-row-desc">{{ pgpKeySubtitle(key) }}</span>
          </div>
          <div class="km-xp-row-actions">
            <button
              type="button"
              class="km-xp-btn km-xp-btn-outline"
              @click="toggleTrust(key.id)"
            >
              {{ key.trusted ? "已信任" : "信任" }}
            </button>
            <button
              type="button"
              class="km-xp-btn km-xp-btn-outline"
              @click="toggleSign(key.id)"
            >
              默认签名: {{ key.signByDefault ? "开" : "关" }}
            </button>
            <button
              type="button"
              class="km-xp-btn km-xp-btn-ghost"
              title="删除登记"
              aria-label="删除密钥登记"
              @click="removeKey(key.id)"
            >
              <SettingsIcon name="trash" :size="14" />
            </button>
          </div>
        </div>

        <!-- 登记表单（GPUI 两行结构） -->
        <div class="km-xp-form">
          <div class="km-xp-form-line">
            <input
              v-model="form.name"
              type="text"
              class="km-xp-input km-xp-flex-1"
              placeholder="名称（可选）"
              aria-label="密钥名称"
            />
            <input
              v-model="form.email"
              type="email"
              class="km-xp-input km-xp-flex-1"
              placeholder="绑定邮箱，例如 user@example.com"
              aria-label="绑定邮箱"
            />
          </div>
          <div class="km-xp-form-line">
            <input
              v-model="form.fingerprint"
              type="text"
              class="km-xp-input km-xp-flex-1 km-xp-mono"
              placeholder="指纹（留空自动生成 LOCAL 标识）"
              aria-label="密钥指纹"
              @keydown.enter.prevent="registerKey"
            />
            <button
              type="button"
              class="km-xp-btn km-xp-btn-primary"
              @click="registerKey"
            >
              <SettingsIcon name="plus" :size="13" />
              登记密钥
            </button>
          </div>
        </div>
      </div>
    </section>
  </div>
</template>

<script setup lang="ts">
import { reactive, ref } from "vue";
import PillSwitch from "./PillSwitch.vue";
import SettingsIcon from "./SettingsIcon.vue";
import {
  pgpKeySubtitle,
  pgpKeyTitle,
  registerPgpKey,
  type PgpKeyItem,
} from "./extraTabsLogic";

/**
 * 「OpenPGP 密钥」页（GPUI extra_pages.rs#render_openpgp 校准）。
 * - 查询策略（AppPreferences.openpgp_wkd_lookup / openpgp_keyserver_lookup）与密钥列表
 *   （AppPreferences.pgp_keys）均不在现有 Vue 偏好 IPC 契约内：以会话内状态实现
 *   开关 / 登记 / 信任切换 / 默认签名切换 / 删除（见汇报接线缺口）。
 * - 反馈文案沿用 GPUI（明确说明实际加解密待 Windows PGP 引擎接入）。
 */
const keys = ref<PgpKeyItem[]>([]);
const feedback = ref<{ ok: boolean; msg: string } | null>(null);
const wkdLookup = ref(false);
const keyserverLookup = ref(false);

const form = reactive({
  name: "",
  email: "",
  fingerprint: "",
});

function registerKey() {
  const result = registerPgpKey(keys.value, {
    name: form.name,
    email: form.email,
    fingerprint: form.fingerprint,
  });
  if (result.error) {
    feedback.value = { ok: false, msg: result.error };
    return;
  }
  keys.value = result.keys;
  form.name = "";
  form.email = "";
  form.fingerprint = "";
  feedback.value = {
    ok: true,
    msg: "已保存密钥元信息（实际加解密待 Windows PGP 引擎接入）",
  };
}

function toggleTrust(id: number) {
  keys.value = keys.value.map((k) => (k.id === id ? { ...k, trusted: !k.trusted } : k));
}

function toggleSign(id: number) {
  keys.value = keys.value.map((k) => (k.id === id ? { ...k, signByDefault: !k.signByDefault } : k));
}

function removeKey(id: number) {
  keys.value = keys.value.filter((k) => k.id !== id);
}
</script>

<style scoped>
.km-xp-mono {
  font-family: var(--km-font-mono);
  font-size: 12px;
}
</style>

<style scoped src="./extraPageBase.css" />
