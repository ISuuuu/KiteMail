<template>
  <!-- 账号列表层（GPUI accounts.rs#L1438）；ref 供搜索命中时做卡片级高亮（WP-09） -->
  <AccountSettingsList
    v-if="layer === 'list'"
    ref="listRef"
    :id="'sp-account-list'"
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
import { computed, onMounted, onBeforeUnmount, ref, watch } from "vue";
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
    /** 搜索命中信号（递增 token）；变化时对账号列表卡做 1.6s 卡片级高亮 */
    highlightToken?: number;
  }>(),
  { hostChrome: false, highlightToken: 0 }
);

const emit = defineEmits<{
  /** 编辑态顶栏状态（壳层据此渲染标题与 返回/移除/保存） */
  chrome: [chrome: { editorActive: boolean; title: string; canRemove: boolean; busy: boolean }];
  /** 守卫放行后的关窗动作（未保存横幅「放弃更改 / 保存」后触发，WP-10） */
  closeWindow: [];
}>();

const accountState = useAccounts();
const editorRef = ref<InstanceType<typeof AccountSettingsEdit> | null>(null);
const listRef = ref<InstanceType<typeof AccountSettingsList> | null>(null);

// ---- 搜索命中：账号列表卡片级高亮 1.6s（WP-09，不改 AccountSettingsList） ----
const HIGHLIGHT_MS = 1600;
const ACCOUNT_CARD_CLASS = "km-account-card-hit";
let highlightTimer: ReturnType<typeof setTimeout> | null = null;

watch(
  () => props.highlightToken,
  (token) => {
    if (token <= 0) return;
    const el = listRef.value?.$el as HTMLElement | undefined;
    if (!el) return;
    el.scrollIntoView({ block: "nearest" });
    el.classList.add(ACCOUNT_CARD_CLASS);
    if (highlightTimer) clearTimeout(highlightTimer);
    highlightTimer = setTimeout(() => {
      el.classList.remove(ACCOUNT_CARD_CLASS);
      highlightTimer = null;
    }, HIGHLIGHT_MS);
  }
);

onBeforeUnmount(() => {
  if (highlightTimer) clearTimeout(highlightTimer);
  highlightTimer = null;
});

const {
  layer,
  pendingLeave,
  snapshotBaseline,
  startCreateFlow,
  requestBack,
  requestClose,
  onRowSelect,
  discardPendingLeave,
  saveAndLeave,
  onEditorSaved,
  onEditorRemoved,
} = useAccountLayers({
  state: accountState,
  // 关窗离开动作（useAccountLayers 层面）：设置窗壳层负责原生关窗
  onClose: () => emit("closeWindow"),
});

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
  // ---- WP-10：壳层 Esc / Ctrl+W 逐层收合所需（经 useAccountLayers 委托） ----
  /** 未保存守卫横幅是否挂起 */
  pendingLeave,
  /** 取消未保存守卫横幅（GPUI Esc 清 pending_leave_action） */
  cancelPendingLeave() {
    pendingLeave.value = null;
  },
  /** 宿主关窗入口：编辑态脏 → 挂起守卫返回 true（壳层暂不关窗） */
  requestHostClose(): boolean {
    return requestClose();
  },
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
