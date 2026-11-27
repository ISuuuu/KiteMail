<template>
  <div class="km-xp-page">
    <!-- 反馈横幅（保存 / 删除结果） -->
    <div
      v-if="feedback"
      class="km-xp-banner"
      :class="feedback.ok ? 'ok' : 'err'"
      role="status"
    >
      {{ feedback.msg }}
    </div>

    <!-- 自定义标签组（GPUI extra_pages.rs#render_tags） -->
    <section class="km-xp-group" id="sp-tags-page">
      <h4 class="km-xp-group-title">自定义标签（本地会话）</h4>
      <p class="km-xp-group-desc">支持调色板颜色圆点与 1..9 数字快捷键绑定</p>
      <div class="km-xp-card">
        <!-- 空态 / 列表行 -->
        <div v-if="tags.length === 0" class="km-xp-empty">
          尚未创建自定义标签，可在下方添加。
        </div>
        <div
          v-for="tag in tags"
          :key="tag.id"
          class="km-xp-row"
        >
          <div class="km-xp-row-main">
            <span class="km-xp-tag-dot" :style="{ background: tag.color }" />
            <span class="km-xp-row-title">{{ tag.name }}</span>
            <span class="km-xp-tag-pill">{{ tagShortcutLabel(tag.shortcut) }}</span>
          </div>
          <div class="km-xp-row-actions">
            <button
              type="button"
              class="km-xp-btn km-xp-btn-ghost"
              title="编辑标签"
              aria-label="编辑标签"
              @click="startEdit(tag)"
            >
              <SettingsIcon name="pencil" :size="14" />
            </button>
            <button
              type="button"
              class="km-xp-btn km-xp-btn-ghost"
              title="删除标签"
              aria-label="删除标签"
              @click="removeTag(tag.id)"
            >
              <SettingsIcon name="trash" :size="14" />
            </button>
          </div>
        </div>

        <!-- 新增 / 编辑表单 -->
        <div class="km-xp-form">
          <div class="km-xp-form-line">
            <span class="km-xp-flex-1">
              <input
                v-model="tagName"
                type="text"
                class="km-xp-input km-xp-flex-1"
                placeholder="标签名称"
                aria-label="标签名称"
                @keydown.enter.prevent="saveTag"
              />
            </span>
            <ComboSelect
              :options="TAG_SHORTCUT_OPTIONS"
              :modelValue="tagShortcut === null ? '' : String(tagShortcut)"
              title="数字快捷键"
              aria-label="标签快捷键"
              @update:modelValue="onShortcutChange"
            />
          </div>
          <div class="km-xp-form-line">
            <div class="km-xp-palette" role="group" aria-label="标签颜色">
              <button
                v-for="hex in ACCOUNT_ACCENT_COLORS"
                :key="hex[0]"
                type="button"
                class="km-xp-swatch"
                :class="{ selected: tagColor.toLowerCase() === hex[0] }"
                :style="{ background: hex[0] }"
                :title="hex[1]"
                :aria-label="`标签颜色 ${hex[1]}`"
                @click="tagColor = hex[0]"
              />
            </div>
            <span class="km-xp-flex-1" />
            <button
              v-if="editingTagId !== null"
              type="button"
              class="km-xp-btn km-xp-btn-outline"
              @click="cancelEdit"
            >
              取消
            </button>
            <button
              type="button"
              class="km-xp-btn km-xp-btn-primary"
              @click="saveTag"
            >
              <SettingsIcon :name="editingTagId !== null ? 'check' : 'plus'" :size="13" />
              {{ editingTagId !== null ? "保存修改" : "添加标签" }}
            </button>
          </div>
        </div>
      </div>
    </section>
  </div>
</template>

<script setup lang="ts">
import { ref } from "vue";
import ComboSelect from "./ComboSelect.vue";
import SettingsIcon from "./SettingsIcon.vue";
import {
  TAG_SHORTCUT_OPTIONS,
  saveTag as applyTagSave,
  tagShortcutLabel,
  type TagItem,
} from "./extraTabsLogic";
import { ACCOUNT_ACCENT_COLORS } from "../accounts/brand";

/**
 * 「标签」页（GPUI extra_pages.rs#render_tags 校准）。
 * 标签数据域不在现有 Tauri IPC 契约内：以会话内状态实现完整的
 * 新增 / 编辑 / 快捷键绑定 / 删除交互；刷新后不持久化（见汇报接线缺口）。
 */
const tags = ref<TagItem[]>([]);
const editingTagId = ref<number | null>(null);
const tagName = ref("");
const tagColor = ref("#3584e4");
const tagShortcut = ref<number | null>(null);
const feedback = ref<{ ok: boolean; msg: string } | null>(null);

function onShortcutChange(value: string) {
  tagShortcut.value = value === "" ? null : Number(value);
}

function startEdit(tag: TagItem) {
  editingTagId.value = tag.id;
  tagName.value = tag.name;
  tagColor.value = tag.color;
  tagShortcut.value = tag.shortcut;
}

function cancelEdit() {
  editingTagId.value = null;
  tagName.value = "";
  tagShortcut.value = null;
}

function saveTag() {
  const result = applyTagSave(tags.value, {
    id: editingTagId.value,
    name: tagName.value,
    color: tagColor.value,
    shortcut: tagShortcut.value,
  });
  if (result.error) {
    feedback.value = { ok: false, msg: result.error };
    return;
  }
  const wasEditing = editingTagId.value !== null;
  tags.value = result.tags;
  editingTagId.value = null;
  tagName.value = "";
  tagShortcut.value = null;
  feedback.value = {
    ok: true,
    // GPUI 文案为「已保存到 SQLite tags 表」；本页为会话内状态，降级为不含存储承诺的提示。
    msg: wasEditing ? "已保存标签修改" : "已保存标签",
  };
}

function removeTag(id: number) {
  tags.value = tags.value.filter((t) => t.id !== id);
  if (editingTagId.value === id) {
    editingTagId.value = null;
    tagName.value = "";
  }
  feedback.value = { ok: true, msg: "已删除标签" };
}
</script>

<style scoped src="./extraPageBase.css" />
