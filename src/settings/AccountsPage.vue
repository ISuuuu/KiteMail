<template>
  <!-- 账号列表层（GPUI accounts.rs#L1438） -->
  <AccountSettingsList
    v-if="layer === 'list'"
    class="km-accounts-layer"
    :state="accountState"
    @add="startCreateFlow"
    @select="onRowSelect"
    @toggle="(id, enabled) => void accountState.toggleAccountEnabled(id, enabled)"
    @reorder="onReorder"
    @retry="accountState.loadAccounts()"
  />
  <!-- 账号编辑层：顶栏承载返回（先回列表）/ 移除 / 保存（GPUI accounts.rs#L1801）；
       hostChrome=true 时由壳层内容顶栏承载（WP-07 单顶栏） -->
  <AccountSettingsEdit
    v-else
    ref="editorRef"
    class="km-accounts-layer"
    :state="accountState"
    :editingId="accountState.isCreating.value ? null : accountState.selectedAccountId.value"
    :pendingLeave="pendingLeave !== null"
    :hostChrome="hostChrome"
    @back="requestBack"
    @saved="onEditorSaved"
    @removed="onEditorRemoved"
    @leave-cancel="pendingLeave = null"
    @leave-discard="discardPendingLeave"
    @leave-save="saveAndLeave"
  />
</template>

<script setup lang="ts">
import { computed, onMounted, ref, watch } from "vue";
import AccountSettingsEdit from "../accounts/AccountSettingsEdit.vue";
import AccountSettingsList from "../accounts/AccountSettingsList.vue";
import { useAccountLayers } from "../accounts/useAccountLayers";
import { useAccounts } from "../accounts/useAccounts";

/**
 * 设置窗「邮箱账号」页（WP-04）：在设置窗右侧承载账号列表与编辑两级层级。
 * - 设置窗内自建 useAccounts 实例，以 state prop 模式传入账号组件（不重复实现 IPC）；
 * - 层级切换 / 未保存守卫 / 保存后回列表与独立账号弹窗共用 useAccountLayers 编排；
 * - 编辑「返回」先回账号列表，列表态由壳层内容顶栏「返回」回导航（窄窗 GPUI 行为）；
 * - hostChrome（WP-07）：壳层承载编辑态单顶栏，页面向上抛 chrome 状态并暴露动作。
 */
const props = withDefaults(
  defineProps<{
    /** 壳层承载编辑顶栏（返回/移除/保存上提），隐藏编辑组件内置顶栏 */
    hostChrome?: boolean;
  }>(),
  { hostChrome: false }
);

const emit = defineEmits<{
  /** 编辑态顶栏状态（壳层据此渲染标题与 返回/移除/保存） */
  chrome: [chrome: { editorActive: boolean; title: string; canRemove: boolean; busy: boolean }];
}>();

const accountState = useAccounts();
const editorRef = ref<InstanceType<typeof AccountSettingsEdit> | null>(null);

const {
  layer,
  pendingLeave,
  snapshotBaseline,
  startCreateFlow,
  requestBack,
  onRowSelect,
  discardPendingLeave,
  saveAndLeave,
  onEditorSaved,
  onEditorRemoved,
} = useAccountLayers({ state: accountState });

onMounted(() => {
  void accountState.init().then(() => snapshotBaseline());
});

// 编辑态 chrome 状态上抛（供壳层单顶栏渲染）
const chromeState = computed(() => ({
  editorActive: layer.value === "editor",
  title: editorRef.value?.editorTitle ?? "编辑账号",
  canRemove: editorRef.value?.canRemove ?? false,
  busy: editorRef.value?.editorBusy ?? false,
}));
watch(
  chromeState,
  (value) => {
    emit("chrome", value);
  },
  { deep: true, immediate: true }
);

/** 壳层「返回」：编辑态先回账号列表（带未保存守卫）。 */
function backToAccountList() {
  requestBack();
}

/** 壳层「移除」：切换内联移除确认横幅。 */
function toggleRemoveConfirm() {
  editorRef.value?.toggleRemoveConfirm();
}

/** 壳层「保存」。 */
function saveFromChrome() {
  void editorRef.value?.requestSave();
}

defineExpose({
  layer,
  backToAccountList,
  toggleRemoveConfirm,
  saveFromChrome,
});

// 拖拽排序暂无持久化 IPC（Tauri 命令清单未含 reorder），把手仅作视觉呈现；后端支持后在此接线。
function onReorder(_fromIndex: number, _toIndex: number) {
  /* 预留接线点 */
}
</script>

<style scoped>
/*
 * 容器适配：壳层 .km-set-content-scroll 自带 20px/24px 内边距与滚动，
 * 账号两级组件自带 GPUI px_6 py_5（20px/24px）内边距与内部滚动。
 * 这里用负外边距抵消壳层内边距，由组件自管内容留白与滚动（对齐 GPUI 内容区基准）。
 * 依赖壳层内边距保持 20px/24px（GPUI 普通内容区基准值）。
 */
.km-accounts-layer {
  flex: 1;
  min-height: 0;
  align-self: stretch;
  margin: -20px -24px;
}
</style>
