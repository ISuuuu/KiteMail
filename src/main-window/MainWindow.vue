<template>
  <div ref="rootRef" class="km-main-window" :class="layoutClass">
    <FolderSidebar
      :accounts="state.accounts.value"
      :folders="state.folders.value"
      :currentView="state.currentView.value"
      :isRail="state.layout.isRail.value"
      :width="state.layout.sidebarVisualWidth.value"
      :syncing="state.syncStatus.value.syncing"
      :mode="state.mode.value"
      :hasPasscode="state.hasPasscode.value"
      @selectView="state.selectView"
      @toggleCollapse="state.layout.toggleSidebarCollapsed"
      @refresh="state.refresh"
      @lock="state.lock"
    />

    <!-- 侧栏分隔把手 -->
    <PaneSplitter
      v-if="!state.layout.isRail.value"
      direction="vertical"
      label="调整侧栏宽度"
      :value="state.layout.prefs.value.sidebarWidth"
      :min="SIDEBAR_MIN_WIDTH"
      :max="SIDEBAR_MAX_WIDTH"
      @dragStart="dragBaseSidebar = state.layout.prefs.value.sidebarWidth"
      @drag="(delta) => state.layout.onSidebarResize(dragBaseSidebar + delta)"
      @dragEnd="state.layout.flushSave()"
      @step="(delta) => state.layout.onSidebarResize(state.layout.prefs.value.sidebarWidth + delta)"
    />

    <!-- 中栏：列表（Right 模式定宽，Below/Off 模式铺满） -->
    <div class="km-list-column" :style="listColumnStyle">
      <div class="km-list-slot" :style="listSlotStyle">
        <MessageListPane
          :messages="state.messages.value"
          :groupedMessages="state.groupedMessages.value"
          :total="state.listTotal.value"
          :loading="state.listLoading.value"
          :error="state.listError.value"
          :hasMore="state.hasMore.value"
          :searchQuery="state.searchQuery.value"
          :searchScope="state.searchScope.value"
          :sortMode="state.sortMode.value"
          :filterSet="state.filterSet.value"
          :selectedId="state.selectedId.value"
          :listWide="state.layout.listWide.value"
          :readerPosition="state.layout.prefs.value.readerPosition"
          :compactList="state.layout.prefs.value.compactList"
          :showAvatars="state.layout.prefs.value.showAvatars"
          :scrollResetToken="state.scrollResetToken.value"
          :syncStatus="state.syncStatus.value"
          :cacheReady="state.cacheReady.value"
          @selectMessage="state.selectMessage"
          @toggleSeen="state.toggleMessageSeen"
          @toggleFlagged="state.toggleMessageFlagged"
          @setSearch="state.setSearch"
          @setScope="state.setScope"
          @setSort="state.setSort"
          @setFilter="state.setFilter"
          @refresh="state.refresh"
          @loadMore="state.loadMore"
          @overlayChange="state.layout.setOverlayOpen"
        />
      </div>

      <!-- Below 模式：列表下方阅读区 -->
      <template v-if="state.layout.prefs.value.readerPosition === 'below'">
        <PaneSplitter
          direction="horizontal"
          label="调整列表与阅读区高度"
          :value="state.layout.prefs.value.listBelowHeight"
          :min="BELOW_LIST_MIN_HEIGHT"
          @dragStart="dragBaseBelow = state.layout.prefs.value.listBelowHeight"
          @drag="(delta) => state.layout.onBelowResize(dragBaseBelow + delta)"
          @dragEnd="state.layout.flushSave()"
          @step="(delta) => state.layout.onBelowResize(state.layout.prefs.value.listBelowHeight + delta)"
        />
        <ReaderPane
          :isBelow="true"
          :message="state.openedMessage.value"
          :loading="state.readerLoading.value"
          :error="state.readerError.value"
          :selectedUnread="state.selectedUnread.value"
          :readerPosition="state.layout.prefs.value.readerPosition"
          :themeId="state.layout.prefs.value.themeId"
          :effectiveDark="state.layout.effectiveDark.value"
          :followSystem="state.layout.followSystemScheme.value"
          :overlayOccluded="state.layout.overlayOccluded.value"
          :zoomPercent="state.layout.prefs.value.readerZoomPercent"
          @retry="state.retryOpen"
          @toggleSeen="(seen) => state.selectedId.value && state.toggleMessageSeen(state.selectedId.value, seen)"
          @toggleFlagged="(flagged) => state.selectedId.value && state.toggleMessageFlagged(state.selectedId.value, flagged)"
          @setReaderPosition="state.layout.setReaderPosition"
          @setTheme="state.layout.setTheme"
          @toggleDarkMode="state.layout.toggleDarkMode"
          @toggleFollowSystem="state.layout.toggleFollowSystem"
          @overlayChange="state.layout.setOverlayOpen"
        />
      </template>
    </div>

    <!-- Right 模式：右侧阅读区 -->
    <template v-if="state.layout.prefs.value.readerPosition === 'right'">
      <PaneSplitter
        direction="vertical"
        label="调整列表栏宽度"
        :value="state.layout.prefs.value.listWidth"
        :min="LIST_MIN_WIDTH"
        @dragStart="dragBaseList = state.layout.prefs.value.listWidth"
        @drag="(delta) => state.layout.onListResize(dragBaseList + delta)"
        @dragEnd="state.layout.flushSave()"
        @step="(delta) => state.layout.onListResize(state.layout.prefs.value.listWidth + delta)"
      />
      <ReaderPane
        class="km-reader-column"
        :message="state.openedMessage.value"
        :loading="state.readerLoading.value"
        :error="state.readerError.value"
        :selectedUnread="state.selectedUnread.value"
        :readerPosition="state.layout.prefs.value.readerPosition"
        :themeId="state.layout.prefs.value.themeId"
        :effectiveDark="state.layout.effectiveDark.value"
        :followSystem="state.layout.followSystemScheme.value"
        :overlayOccluded="state.layout.overlayOccluded.value"
        :zoomPercent="state.layout.prefs.value.readerZoomPercent"
        @retry="state.retryOpen"
        @toggleSeen="(seen) => state.selectedId.value && state.toggleMessageSeen(state.selectedId.value, seen)"
        @toggleFlagged="(flagged) => state.selectedId.value && state.toggleMessageFlagged(state.selectedId.value, flagged)"
        @setReaderPosition="state.layout.setReaderPosition"
        @setTheme="state.layout.setTheme"
        @toggleDarkMode="state.layout.toggleDarkMode"
        @toggleFollowSystem="state.layout.toggleFollowSystem"
        @overlayChange="state.layout.setOverlayOpen"
      />
    </template>

    <!-- Off 模式浮动入口：允许在隐藏阅读区时切回阅读位置或主题 -->
    <div v-if="state.layout.prefs.value.readerPosition === 'off'" class="km-off-bar">
      <button
        type="button"
        class="km-off-chip"
        title="恢复右侧阅读区 (Ctrl+Alt+1)"
        @click="state.layout.setReaderPosition('right')"
      >
        显示阅读窗格
      </button>
    </div>
  </div>
</template>

<script setup lang="ts">
import { computed, onMounted, onUnmounted, ref } from "vue";
import FolderSidebar from "./FolderSidebar.vue";
import MessageListPane from "./MessageListPane.vue";
import ReaderPane from "./ReaderPane.vue";
import PaneSplitter from "./PaneSplitter.vue";
import { injectMainWindow } from "./useMainWindow";
import {
  BELOW_LIST_MIN_HEIGHT,
  LIST_MIN_WIDTH,
  SIDEBAR_MAX_WIDTH,
  SIDEBAR_MIN_WIDTH,
} from "./usePaneLayout";

const state = injectMainWindow();
const rootRef = ref<HTMLElement | null>(null);

const dragBaseSidebar = ref(state.layout.prefs.value.sidebarWidth);
const dragBaseList = ref(state.layout.prefs.value.listWidth);
const dragBaseBelow = ref(state.layout.prefs.value.listBelowHeight);

let ro: ResizeObserver | null = null;

onMounted(() => {
  if (rootRef.value) {
    const rect = rootRef.value.getBoundingClientRect();
    if (rect.width > 0) {
      state.layout.setWindowSize(rect.width, rect.height);
    }
    ro = new ResizeObserver((entries) => {
      const cr = entries[0].contentRect;
      state.layout.setWindowSize(cr.width, cr.height);
    });
    ro.observe(rootRef.value);
  }
  window.addEventListener("keydown", onKeyDown);
});

onUnmounted(() => {
  ro?.disconnect();
  window.removeEventListener("keydown", onKeyDown);
  state.layout.flushSave();
});

const listColumnStyle = computed(() => {
  const prefs = state.layout.prefs.value;
  if (prefs.readerPosition === "right") {
    return {
      width: `${prefs.listWidth}px`,
      minWidth: `${LIST_MIN_WIDTH}px`,
      flexShrink: 0,
    };
  }
  return {
    flex: "1 1 auto",
    minWidth: `${LIST_MIN_WIDTH}px`,
  };
});

const listSlotStyle = computed(() => {
  const prefs = state.layout.prefs.value;
  if (prefs.readerPosition === "below") {
    return {
      height: `${prefs.listBelowHeight}px`,
      minHeight: `${BELOW_LIST_MIN_HEIGHT}px`,
      flexShrink: 0,
      display: "flex",
      flexDirection: "column" as const,
      overflow: "hidden",
    };
  }
  return {
    flex: "1 1 auto",
    minHeight: "0",
    display: "flex",
    flexDirection: "column" as const,
    overflow: "hidden",
  };
});

const layoutClass = computed(() => ({
  "km-layout-right": state.layout.prefs.value.readerPosition === "right",
  "km-layout-below": state.layout.prefs.value.readerPosition === "below",
  "km-layout-off": state.layout.prefs.value.readerPosition === "off",
  "km-rail": state.layout.isRail.value,
}));

function isEditableFocused(): boolean {
  const el = document.activeElement as HTMLElement | null;
  if (!el) return false;
  if (el.isContentEditable) return true;
  return el.tagName === "INPUT" || el.tagName === "TEXTAREA" || el.tagName === "SELECT";
}

function onKeyDown(e: KeyboardEvent) {
  // 中文 IME 组合输入期间保持编辑键优先
  if (e.isComposing || e.key === "Process" || e.keyCode === 229) return;

  // Ctrl+F: 聚焦搜索框
  if (e.ctrlKey && !e.altKey && !e.shiftKey && e.key.toLowerCase() === "f") {
    e.preventDefault();
    window.dispatchEvent(new CustomEvent("km-focus-search"));
    return;
  }

  // F5: 刷新
  if (!e.ctrlKey && !e.altKey && e.key === "F5") {
    e.preventDefault();
    void state.refresh();
    return;
  }

  // Ctrl+Alt+1/2/3: 切换阅读位置 right/below/off
  if (e.ctrlKey && e.altKey) {
    if (e.key === "1" || e.code === "Digit1") {
      e.preventDefault();
      state.layout.setReaderPosition("right");
      return;
    }
    if (e.key === "2" || e.code === "Digit2") {
      e.preventDefault();
      state.layout.setReaderPosition("below");
      return;
    }
    if (e.key === "3" || e.code === "Digit3") {
      e.preventDefault();
      state.layout.setReaderPosition("off");
      return;
    }
  }

  // 上/下键选邮件（输入控件聚焦时不拦截）
  if (isEditableFocused()) return;
  if (!e.ctrlKey && !e.altKey && !e.shiftKey) {
    if (e.key === "ArrowUp") {
      e.preventDefault();
      state.selectPrev();
      return;
    }
    if (e.key === "ArrowDown") {
      e.preventDefault();
      state.selectNext();
      return;
    }
  }
}
</script>

<style scoped>
.km-main-window {
  width: 100%;
  height: 100%;
  display: flex;
  flex-direction: row;
  overflow: hidden;
  background: var(--km-canvas);
  position: relative;
}

.km-list-column {
  display: flex;
  flex-direction: column;
  overflow: hidden;
  min-width: 0;
  height: 100%;
}

.km-reader-column {
  flex: 1 1 auto;
  min-width: min(441px, 100%);
  overflow: hidden;
}

.km-off-bar {
  position: absolute;
  right: 14px;
  bottom: 14px;
  z-index: 15;
}

.km-off-chip {
  padding: 6px 12px;
  border-radius: 999px;
  border: 1px solid var(--km-border);
  background: var(--km-surface-raised);
  box-shadow: var(--km-shadow-card);
  font-size: 12px;
  cursor: pointer;
}

.km-off-chip:hover {
  border-color: var(--km-accent);
  color: var(--km-accent);
}
</style>
