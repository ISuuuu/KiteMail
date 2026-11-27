# KiteMail 主窗口 Tauri 和 Vue 迁移实施方案

## 1. 交付目标

KiteMail 的目标架构是 **Rust 邮件核心 + Tauri 2 桌面宿主 + Vue 3 / TypeScript / Vite 界面**。首期先迁移 Windows x64 主窗口。原有产品布局和交互作为验收基线，邮件正文继续使用 WebView2 的独立阅读环境。

| 里程碑 | 可交付结果 | 数据范围 |
| --- | --- | --- |
| A. 窗口和阅读器原型 | 独立启动 Tauri 窗口，验证同窗 WebView2 的位置、焦点、层级和隔离 | 固定的本地 HTML 样本 |
| B. 主窗口预览 | `--demo` 下能切文件夹、搜索、滚动翻页和阅读邮件，支持原有分栏与主题 | `DemoStore` 演示数据 |
| C. 主窗口接入真实邮件 | 离线先显示缓存，后台同步，按需加载正文，按原规则标记已读，支持锁屏和托盘 | 独立测试资料目录和测试账号 |
| D. 正式替换入口 | 写信、设置、账号管理等入口迁移完成，安装、便携和更新链路验收通过 | 现有安装版和便携版数据 |

A、B 是第一批实现范围。C 是主窗口的第二批实现。正式发行继续从 GPUI 入口构建，直到 D 的切换条件全部满足。正文挂载原型的结论决定 B 使用同窗 HTML 阅读或主窗纯文本加独立 HTML 阅读窗口。

## 2. 目录和依赖

以下为目标目录。`crates/windows-mail-reader` 从现有阅读器中逐步提取，Tauri 和 GPUI 共用。目录内只创建当前批次需要的文件。

```text
crates/
  mail-core/                          现有协议、缓存、存储、凭据和后台 Worker
  app/                                现有 GPUI 正式客户端
    src/webview_host.rs               保留 GPUI 专属的阅读器适配层
  windows-mail-reader/                Windows 原生阅读器，由两个宿主共用
    src/lib.rs                        暴露少量宿主接口及值类型
    src/document.rs                   单封正文文档与主题样式
    src/thread_document.rs            会话文档，沿用现有构建器
    src/dark.rs                       邮件正文深色适配
    src/policy.rs                     来源、导航、远程资源和链接策略
    src/webview2.rs                   WebView2 控制器、事件回调、生命周期
    src/scripts/                      现有可信阅读器脚本
package.json                        Vue、Vite、Tauri CLI 与构建脚本
pnpm-lock.yaml
vite.config.ts
src/
  main.ts
  App.vue                           挂载 MainWindow，管理应用级错误页
  main-window/
    MainWindow.vue                  组装侧栏、列表、阅读区和状态条
    FolderSidebar.vue               文件夹树、未读数、选中态
    MessageListPane.vue             搜索、筛选、虚拟列表、列表状态
    MessageRow.vue                  行显示和键盘焦点
    ReaderPane.vue                  邮件头、正文占位及重试状态
    ReaderViewport.vue              测量原生正文区域，报告可见性
    PaneSplitter.vue                鼠标和键盘分栏调整
    useMainWindow.ts                当前视图、列表请求、选择与交互
    usePaneLayout.ts                布局宽度、阅读位置、偏好持久化
  ipc/
    main-window.ts                  类型化 invoke、事件订阅和错误映射
    types.ts                        前端 DTO 声明
  styles/theme.css                  现有主题语义色和布局变量
src-tauri/
  Cargo.toml                        独立 Cargo 工作区
  Cargo.lock
  build.rs                          Tauri 构建和自定义命令权限声明
  tauri.conf.json
  capabilities/main.json
  src/
    main.rs                         启动参数、Tauri 注册与 setup
    commands.rs                     IPC 参数校验与服务调用
    dto.rs                          IPC 输入、输出和错误定义
    mail_session.rs                 演示或真实会话，Worker 和后台事件
    reader.rs                       Tauri 到原生阅读器的主线程适配
    preferences.rs                  启动偏好和布局字段读写
    window_lifecycle.rs             窗口几何、单实例、托盘、退出
```

- `crates/app`、`crates/mail-core` 和 `crates/windows-mail-reader` 位于根 Cargo 工作区。`src-tauri` 作为独立 Cargo 工作区，以路径依赖引用两个 Rust 库。根 `cargo check --workspace` 继续只走现有 Rust 构建链及共享阅读器，前端构建加入单独的 Tauri Windows CI 检查。在只有实际代码需要共享时再增加 Rust 公共模块。
- `mail-core` 保持与界面框架无关。两个客户端各自调用 Rust 库。生产 Vue 静态产物随 Tauri 一起打包，邮件功能通过本地 IPC 接入 Rust。第一期用 Vue 组件和组合式函数管理一个主窗口的状态，写信等独立窗口出现后再评估跨窗口需求。 [Tauri 与 Vite 配置](https://v2.tauri.app/start/frontend/vite/)
- `package.json` 提供 `build`（`vue-tsc --noEmit && vite build`）和 `tauri`（`tauri`）脚本。`tauri.conf.json` 使用 `frontendDist: "../dist-web"` 和 `beforeBuildCommand: "pnpm build"`。将根目录 `node_modules`、`dist-web` 和 `src-tauri/target` 排除在版本控制之外。 [Tauri 与 Vite 构建配置](https://v2.tauri.app/start/frontend/vite/)
- 预览二进制命名为 `kitemail-tauri-preview`，使用独立的 Tauri 应用标识、资料目录、WebView2 用户数据目录和单实例范围。`pnpm-lock.yaml` 和 Tauri 的 `Cargo.lock` 纳入版本控制。预览开发使用 Windows MSVC 工具链和 WebView2。 [Tauri Windows 开发环境](https://v2.tauri.app/start/prerequisites/)
- 开发启动脚本为每个 worktree 自动选取空闲的本机回环端口，并将同一个端口同步传给 Tauri `devUrl`、Vite 和冒烟检查。Vite 开启 `strictPort`，端口被抢占时重新选取后整体重启。生产和真实邮件预览使用打包后的前端。 [Tauri 开发配置](https://v2.tauri.app/develop/)

## 3. 现有代码拆分映射

下表按现有实现中的文件和函数定位。优先移动能被两个宿主直接共用的代码。`HylkiApp` 是 GPUI 的状态入口，迁移期间保留它和现有测试作为行为对照。

| 现有位置 | 目标位置 | 具体迁移内容 |
| --- | --- | --- |
| `main.rs` 的 `main`、`resolve_window_size`，`app.rs` 的 `save_window_geometry`、`capture_window_geometry` | Tauri `main.rs`、`window_lifecycle.rs` | 读取现有窗口宽高、最大化和启动全屏字段，使用 Tauri 原生标题栏。保存正常窗口尺寸及最大化标志，避免用最大化尺寸覆盖正常尺寸。 |
| `HylkiApp::new` 中的演示初始化、账号及偏好加载、`spawn_with_notifier` 事件泵 | `mail_session.rs`、`preferences.rs` | A/B 只启动 `DemoStore`。C 启动一个 `MailWorkerHandle`，订阅一个通知源，由一个消费者调用 `drain_events`。初始化提供快照，Worker 状态以小事件推送给窗口。 |
| `CurrentView`、`select_unified_view`、`select_folder`、`remember_last_location`、`ui/sidebar.rs` | `FolderSidebar.vue`、`useMainWindow.ts`、`preferences.rs` | Vue 管理展开和选中态。桥接层保留统一视图键、账号与文件夹 ID 映射，恢复上次位置。Rust 写回既有偏好。旧文件夹不存在时回到统一收件箱。 |
| `reload_current_page`、`send_page_request`、`trigger_load_more`、`maybe_drive_backfill`、`ui/message_list.rs` | `MessageListPane.vue`、`MessageRow.vue`、`useMainWindow.ts`、`mail_session.rs` | Vue 负责虚拟滚动、搜索框、排序和何时请求下一页。Rust 负责账号范围、过滤和数据分页。B 实现演示本地分页。C 接入真实缓存和优先本地、再补拉远端的语义，远端回填由单一协调器负责。 |
| `apply_worker_events` 的文件夹、分页、正文与同步分支 | `mail_session.rs`、`dto.rs` | 提取主窗口使用的状态。同步事件标记有关列表失效，正文响应按邮件标识核对。设置窗口、编辑器、通知等专属事件随对应功能迁移。 |
| `select_message`、`set_selected_message`、`clear_selected_message`、`schedule_delayed_read_mark`、`mark_message_seen_now` | `useMainWindow.ts`、`mail_session.rs`、Tauri `reader.rs` | 保留正文占位、异步拉取、重试、切换时清理旧阅读器和已读策略。Rust 从受信任缓存解析已有的 UID、文件夹路径，负责执行 `SetSeen`。 |
| `ReaderPosition`、`render_mail_pane`、`render_mail_pane_below`、`persist_layout_prefs` | `MainWindow.vue`、`PaneSplitter.vue`、`usePaneLayout.ts`、`preferences.rs` | 将 GPUI 布局计算迁为 CSS Grid 和布局状态。通道沿用 `right`、`below`、`off`。拖拽结束后防抖保存，关闭时冲刷尚未保存的布局字段。 |
| `ui/reader.rs`、`ensure_reader_webview`、`handle_thread_webview_action` | `ReaderPane.vue`、`ReaderViewport.vue`、Tauri `reader.rs` | Vue 显示主题、发件人、时间和操作。Rust 负责正文文档、加载状态、WebView2 生命周期和受限阅读动作。会话渲染由原文档构建器继续承担。 |
| `theme.rs` 的 `PALETTES`、`palette_for`、`reader_body_colors` | `theme.css`、Tauri `reader.rs` | 映射现有主题色、文本缩放和阅读背景，支持浅色、深色和跟随系统。匹配相同主题和缩放设置的基线截图。 |
| `single_instance.rs`、`tray.rs`、`win_idle.rs`，`app.rs` 的 `app_lock_tick`、`handle_tray_command`、`quit_app` | `window_lifecycle.rs`、`mail_session.rs` | C 实现关闭到托盘、二次启动唤醒、明确退出及自动锁定。Windows 事件和会话锁状态在 Rust 管理，锁屏界面由 Vue 渲染。 |

现有入口：[主程序](../crates/app/src/main.rs) · [主窗口状态](../crates/app/src/app.rs) · [侧栏](../crates/app/src/ui/sidebar.rs) · [列表](../crates/app/src/ui/message_list.rs) · [阅读区](../crates/app/src/ui/reader.rs) · [主题](../crates/app/src/theme.rs)

### 阅读器文件的拆分顺序

1. **先移动纯文档和策略。** 从 `webview_host.rs` 提取 `build_body_document`、`build_document`、文档响应头、链接与请求校验。将 `reader_thread_doc.rs`、`webview_dark.rs` 和相关测试随实现迁移。`DocumentAppearance` 的颜色边界采用普通 RGB/RGBA 或 CSS 颜色值。GPUI 的 `Hsla` 转换留在适配层。
2. **再移动控制器和事件。** 从 `WebViewHost::for_window` 拆出接受父 `HWND` 的入口。共享库负责 `create_controller`、裁剪区域、可见性、加载错误和关闭清理。保留 `reader_thread.js`、`thread_scroll.js` 的可信桥接及来源校验。宿主接收类型化通知。`AsyncWindowContext`、`SvgRenderer`、`ImageSource`、焦点回传和 GPUI 快照转换由 GPUI 适配层处理。`Rc` 和 COM 对象保持在窗口主线程，IPC 线程只传递消息 ID、几何值和动作。
3. **检查现有调用方。** `ui/reader.rs`、`ui/message_window.rs`、`ui/print_preview.rs`、`ui/accounts.rs` 以及写信预览会调用现有宿主或图片工具。提取后先让原 `webview_host.rs` 保持兼容入口，检查单封阅读、会话阅读、独立阅读、打印和签名预览。共享库和 GPUI 回归通过后再接 Tauri 宿主。

相关源文件：[WebView2 宿主](../crates/app/src/webview_host.rs) · [会话文档](../crates/app/src/reader_thread_doc.rs) · [深色适配](../crates/app/src/webview_dark.rs) · [独立阅读](../crates/app/src/ui/message_window.rs) · [打印预览](../crates/app/src/ui/print_preview.rs)

## 4. 主窗口 IPC 契约

命令名称以 `snake_case` 注册，序列化结构统一使用 `camelCase`。下表是首批接口上限，只有相应阶段实现的命令才注册。Rust 使用明确 DTO，TypeScript 在 `ipc/types.ts` 维护对应类型，并在 Rust 序列化测试中固定关键字段和枚举名。

| 命令 | 输入与输出 | 责任 |
| --- | --- | --- |
| `main_bootstrap` | 返回模式、锁定状态、账号摘要、文件夹摘要、当前视图、布局、同步状态、状态版本 | Vue 建立主窗口。锁定时只返回必要的锁屏和窗口信息。 |
| `list_messages` | 输入 `requestId`、视图、搜索词、作用域、排序、筛选、`offset`、`limit`。返回 `requestId`、页偏移、总数、行摘要和分页状态 | Rust 校验账号及文件夹关系，限制单页大小。列表响应只带渲染字段。 |
| `open_message` | 输入 `requestId` 和 `{ accountId, messageId }`。返回邮件头、正文状态、必要时的纯文本和 `readerSessionId` | Rust 先查缓存并按需取正文。HTML 由 Rust 交给独立阅读器。 |
| `set_reader_layout` | 输入 `readerSessionId`、递增 `layoutRevision`、内容矩形、裁剪矩形、可见状态 | 在 UI 线程更新位置和可见性。丢弃过期会话或旧几何版本。 |
| `update_main_preferences` | 接收 `readerPosition`、栏宽、下方列表高度等白名单字段的补丁 | Rust 校验数值范围，串行合并并保存到现有偏好文件。 |
| `refresh_mail` | 输入目标视图，返回已接受或错误。通过状态事件反馈进度 | C 接入现有同步请求。B 刷新当前演示列表。 |

- 视图用带 `kind` 的联合类型。例如 `{ kind: "unifiedInbox" }`、`{ kind: "accountFolder", accountId: 2, folderId: 17 }`。邮件引用统一为 `{ accountId, messageId }`。搜索作用域沿用当前文件夹和跨文件夹两类。正文状态至少包含 `loading`、`ready`、`unavailable`，并与当前 `readerSessionId` 一起使用。
- 推送限定在 `main` WebView，包含状态版本和必要的 ID。`main://mail-changed` 通知文件夹或列表失效，`main://sync-status` 通知同步状态，`main://reader-status` 关联阅读会话，`main://lock-state` 通知锁定或解锁。先订阅再取快照，依据状态版本处理启动期间的变化。高频布局更新不使用广播事件。 [Tauri 前端事件与 Channel](https://v2.tauri.app/develop/calling-frontend/)
- 错误 DTO 使用稳定的 `code`、可显示的 `message`、`retryable`，以及有请求上下文时的 `requestId`。覆盖非法参数、锁定、找不到邮件、离线正文、渲染失败。日志使用类别和关联 ID，避免记录正文、凭据和 IPC 原始载荷。
- 命令输入限于业务所需的 ID、枚举、数值和搜索词。文件夹归属、选中邮件和当前会话由 Rust 核验。应用锁、远程图片授权等敏感动作在对应阶段增加专用命令。通过 Tauri `AppManifest::commands` 声明自定义命令，权限绑定到受信任的 `main` WebView，因为注册自定义命令后默认面向所有应用窗口和 WebView。 [Tauri Capabilities](https://v2.tauri.app/security/capabilities/)

### 查询与并发

- **列表。** Vue 每次切视图、切搜索或排序时建立新的查询代次，重置选择和分页。相同查询只允许一个同偏移分页请求在途。归并页时校验查询代次和偏移，按邮件标识去重。边滚动边同步导致顺序变化时，重新请求当前可见范围。查询的空结果和错误都结束当前加载状态。
- **Worker 接入。** 现有 `MailPage` 只有文件夹、偏移、总数和结果。`MessagesPageLoaded` 既用于查询返回，也用于后台更新。C 阶段增加一条带单次回复通道的列表查询入口，共用现有 `LoadUnifiedInbox`、`LoadUnifiedView`、`LoadMessagesPage` 和 `SearchAllFolders` 的查询逻辑。单次回复发送器放在 Worker 内部请求包装结构中，保留 `MailRequest` 的 `Clone` 和 GPUI 事件路径。Tauri 会话任务独占包含事件接收端的 `MailWorkerHandle`，命令层只持有可共享的发送端和必要快照。后台分页事件只触发列表失效，避免用 FIFO 将无标识的广播事件配给前端请求。
- **正文。** Rust 先返回邮件头和正文状态。对相同账号和邮件的重复请求复用在途获取。成功或失败时检查账号、邮件、阅读会话和锁定代次，再更新窗口。遇到当前 Worker 提前返回且没有完成事件的分支，由桥接层提供有界等待与可重试状态。A→B→A 快切时可以复用已填充的缓存，旧会话的加载结果只能更新其对应的缓存。
- **已读。** 沿用 `read_mark_mode` 和现有会话规则。`shown` 在有效阅读选择建立后处理，`delay` 沿用两秒等待和选择代次检查。阅读窗格关闭、切换邮件或锁屏时取消相应的自动已读任务。真实测试验证折叠会话只处理既有规则允许的同文件夹未读邮件。`SetSeen` 成功和失败反馈进入主窗口状态。

现有契约：[MailRequest、MailPage 和 WorkerEvent](../crates/mail-core/src/lib.rs) · [分页与正文实现](../crates/mail-core/src/worker.rs) · [已读与会话选择](../crates/app/src/app.rs)

## 5. 阅读器挂载和隔离

**首选方案**是主窗口使用稳定的 Tauri `WebviewWindow` 加载 Vue，邮件正文由共享库创建的原生 WebView2 控制器承载。A 阶段验证能否将原生控件正确挂到 Tauri 窗口。Tauri 的 `Window::add_child` 属于 `unstable` API，原型优先沿用当前 Windows HWND 方式。 [Tauri 多 WebView API](https://docs.rs/crate/tauri/latest/source/src/lib.rs) · [Windows WebviewWindow 接口](https://docs.rs/tauri/latest/x86_64-pc-windows-msvc/tauri/webview/struct.WebviewWindow.html)

1. `ReaderViewport.vue` 用 `ResizeObserver` 和可见状态监听测量正文区。布局、滚动裁剪、窗口缩放与 DPI 变化合并为每帧最多一次更新。前端传递约定的 CSS 像素，Rust 统一换算为 HWND 客户区物理坐标。明确 WebView 页面缩放与 DPI 的换算来源，切换显示器时校验边界，避免重复乘倍率。
2. 原生控件有独立层级。菜单、弹窗、窄轨浮层会覆盖阅读区时，先在 Rust UI 线程隐藏控件，收到完成反馈后再显示相应浮层。浮层关闭后按当前文档与可见区域恢复。切信、隐藏阅读区、锁屏和窗口退出时，由 Rust 立即隐藏对应控件，并按场景卸载或暂停。原型先用隐藏策略验证流程，共享库保留 GPUI 现有快照行为。
3. 主界面的 Vue DOM 只显示可信界面和转义后的邮件字段。HTML、内联图片和会话文档走专用阅读器。沿用正文沙箱、文档 URL/版本校验、CSP、导航和弹窗拦截、外链校验与系统浏览器打开。远程图片默认拦截，按现有授权范围放行请求并移除 `Referer`、`Cookie`、`Authorization` 等头。继续复用现有禁脚本约束和可信包装层桥接，拒绝来自正文子框架及旧文档的动作。 [WebView2 安全指导](https://learn.microsoft.com/en-us/microsoft-edge/webview2/concepts/security)
4. 阅读器与主界面使用隔离的 WebView2 用户数据目录和权限。仅允许内部阅读器文档和沙箱帧在正文控件内导航。主界面 CSP 的生产配置收紧到打包资源及其必要的 Tauri IPC 来源，Vite 热更新所需来源仅在开发配置启用。阅读控件初始化失败或进程崩溃时，当前窗格提示重试并提供转义后的纯文本。
5. **同窗原型的通过条件**包括分栏连续拖拽、菜单遮挡、键盘能从正文返回邮件列表，以及 Windows 10/11 的 100%、125%、150% DPI 和跨屏重新定位。关键问题无法修复时，B 改为主窗纯文本并提供独立的原生 HTML 阅读窗口，保留相同文档策略及 IPC 契约。第一批报告要记录采用的路径和失败的具体场景。

## 6. 主窗口交互与持久化

- 默认三栏。沿用 `app.rs` 中的布局基线作为 CSS 逻辑像素初始值，侧栏宽度 180–280，中栏最小 280，右侧阅读区最小 441。窗口宽度不超过 1094 时进入图标窄轨。回到宽窗时恢复用户的手动折叠选择。下方布局保持列表与阅读区域的最小高度，窗口继续缩小时允许切换布局或收起阅读区。
- 列表使用稳定邮件 ID 作为行键，虚拟化只渲染视口及附近行。切换文件夹或搜索后回到首屏。同步刷新尽量保持当前邮件和可见滚动锚点。点击行和上下键使用同一选择逻辑。支持 `Ctrl+F`、`F5`、上下键及 `Ctrl+Alt+1/2/3`，并在搜索输入和中文 IME 组合输入期间让编辑键优先。保留清晰的焦点样式。
- 复用 `AppPreferences` 的 `theme_id`、`dark_mode`、窗口尺寸、`window_maximized`、`reader_position`、`sidebar_width`、`list_width`、`list_below_height` 等字段。未知阅读布局键回落 `right`。布局调整沿用约 600 毫秒防抖，窗口关闭时保存最后一次有效值。主题只变更显示状态，当前邮件与列表位置随之保留。
- C 阶段正式读取测试资料前，Rust 核验配置目录及**实际解析后的邮件目录**均在测试范围。自定义绝对路径需指向测试副本。复用 `KITEMAIL_DATA_DIR` 对凭据命名空间的隔离，准备资料时由旧客户端在同一测试目录下建立测试账号，退出后再切到 Tauri。Worker 启动时会检查到期发件箱，测试资料使用专用账号，避免带入真实队列。应用锁、托盘唤醒和退出验证使用同一隔离范围。同一个资料目录只允许一个运行中的客户端负责写入。正式切换时复用旧版命名互斥量或兼容的互斥机制，防止两版同时写入正式数据。 [Worker 启动和定时任务](../crates/mail-core/src/worker.rs) · [旧版单实例](../crates/app/src/single_instance.rs)

现有行为基线：[窗口布局与快捷键](../crates/app/src/app.rs) · [快捷键注册](../crates/app/src/main.rs) · [配置与目录解析](../crates/mail-core/src/config.rs) · [凭据目录隔离](../crates/mail-core/src/secrets.rs)

## 7. 实现批次和验收

### A. 独立入口和原型

1. 搭建 `apps/desktop`。新入口使用 `--demo`，窗口使用原生标题栏，并通过 Tauri/Rust 往返读取一条演示数据。开发进程和预览标识使用前述隔离配置。
2. 提取阅读器必要的文档与原生宿主代码，移动对应测试。用固定样本验证普通正文、长邮件、会话、内联图片、远程图片阻断及外链出站。完成第 5 节的同窗判断。
3. 验收产物是 Windows 上实际运行的预览入口、挂载方案结论、GPUI 回归结果。通过后开始 B。

### B. 演示主窗口

1. 接 `DemoStore` 的文件夹、摘要、搜索、分页和正文。完成三栏组件、阅读窗格位置、主题、状态反馈和首批快捷键。预览版内仅呈现已接通的入口。
2. 使用 `DemoStore` 原有的 10,000 封收件箱样本，测试从首屏连续滚动翻页、搜索后清空、连续切选邮件、键盘选择以及空结果和重试。记录同一台机器上 GPUI 与 Tauri 的冷启动到首屏、滚动表现和运行内存，使用相同样本和测试操作。
3. 给乱序列表响应、切换邮件后旧正文返回、关闭窗口后的尾部布局持久化、禁用阅读窗格时选信等边界增加针对性验证。预览产物验收以打包后的 Windows 可执行文件为准。

### C. 真实测试账号

1. 在后台任务用 `CacheStore::open_for_data_dir_read_only` 尝试读取首屏缓存。已有缓存时先显示结果，缺少缓存时显示初次加载状态，由 Worker 完成初始化。随后接入一个 Worker 和同步流程，首屏避免等待远端网络。实现请求关联、在途正文去重、错误状态、手动刷新和自动刷新。同步变更通过状态事件更新列表。复用 Worker 已有的协议和存储逻辑。
2. 补齐锁屏与解锁流程、关闭到托盘、显式退出和单实例唤醒。锁定时 Rust 拒绝敏感命令、清理当前阅读会话并隐藏正文控件，Vue 清除已加载的邮件视图。后台事件在锁态只推送允许显示的状态。
3. 用断网再恢复、慢响应、跨账号相似文件夹、后台同步期间搜索、旧目录副本、锁屏期间正文延迟返回、重启恢复位置等场景验收。附带既有 GPUI 行为对照和自动化检查结果。

### D. 迁完其余窗口后切换正式发行

按下表接回主窗口的其余入口。每个窗口沿用同一 Rust 会话，界面通过各自的最小命令权限访问。

| 后续功能 | 来源定位 | 拆分边界 |
| --- | --- | --- |
| 写信、草稿、发件箱 | `ui/compose.rs`、`ui/compose_window.rs`，`app.rs` 中的 `schedule_draft_autosave`、`send_compose_inner`、`undo_pending_send_token` | Vue 负责编辑器、收件人和窗口内的编辑状态。Rust 负责草稿写入、发送校验、定时队列和发送撤销。内联及独立窗口共享一份草稿会话。 |
| 设置、账号、首次运行 | `ui/preferences/`、`ui/accounts.rs`、`ui/welcome.rs`，`verify_account_config`、`save_account_config` | Vue 负责表单和提示。账号保存、测试连接、OAuth、凭据迁移及邮件目录切换沿用 Rust 逻辑。 |
| 独立阅读、打印、附件 | `ui/message_window.rs`、`ui/print_preview.rs`、`ui/attachment_drawer.rs`、`ui/gallery.rs` | 复用共享阅读器和正文策略。每个窗口维护自己的阅读选择，打印与保存附件由 Rust 处理已校验的邮件或附件 ID。 |
| 通讯录、命令面板和键盘扩展 | `ui/contacts.rs`、`ui/command_palette.rs`、`main.rs` 的按键绑定 | Vue 负责界面命令和焦点。通讯录存储、导入导出与执行结果继续由 Rust 提供。 |

发行切换时复用现有安装名称、更新目标和数据目录规则。检查现有 NSIS 安装、便携 `portable.flag`、自定义邮件目录、凭据、未发送草稿和定时发送任务的兼容性。验证从现版升级到 Tauri 后，已有资料仍可读取，并用隔离副本验证回滚路径。现有工作流和更新器使用 `kitemail.exe`，切换时需要一起核对二进制名称、更新清单、静默安装和便携包结构。 [Windows 发布流程](../.github/workflows/windows.yml) · [NSIS 脚本](../.github/nsis/kitemail.nsi) · [更新器](../crates/mail-core/src/update.rs)

### 验证命令与交接结果

根目录沿用现有 Windows 验证。新入口的命令在根目录 `package.json` 和两份锁文件落地后执行。

```powershell
cargo fmt --all -- --check
cargo check --workspace --target x86_64-pc-windows-msvc --locked
cargo test --workspace --target x86_64-pc-windows-msvc --locked
git diff --check
```

```powershell
# 在项目根目录执行
pnpm install --frozen-lockfile
pnpm build
cargo fmt --manifest-path src-tauri/Cargo.toml -- --check
cargo test --manifest-path src-tauri/Cargo.toml --target x86_64-pc-windows-msvc --locked
pnpm tauri build --target x86_64-pc-windows-msvc --no-bundle
.\src-tauri\target\x86_64-pc-windows-msvc\release\kitemail-tauri-preview.exe --demo
```

每批交接记录关联文件、实际通过的检查、复现路径和遗留问题。纯前端构建可以验证类型和资源，原生层级、键盘焦点、DPI、隐私隔离和数据兼容以 Windows 运行结果为准。

## 8. 给执行模型的首批任务

在当前仓库实施 **A 和 B**。先读 `crates/app/src/main.rs`、`app.rs` 的启动与事件处理、`webview_host.rs`、`reader_thread_doc.rs`、`crates/mail-core/src/demo.rs`。先搭独立 Tauri 入口并验证阅读器，再按第 3 节抽取共享代码和 Vue 主窗口组件。保留 GPUI 调用入口，用 `--demo` 交付可运行的三栏预览。

首批完成条件是 **选文件夹 → 分页或搜索 → 打开邮件** 全链路可运行，窗口布局和主题可切换，Windows 阅读器挂载路径有实测结论，原 Rust 工作区和 Tauri 构建都有检查记录。真实账号接入从 C 开始。

## 9. 当前迁移进展与 Wave 1 增量契约

### 9.1 里程碑落地状态与目录调整

截至本轮更新，主窗口迁移的 **里程碑 A、B、C 已落地**，正式发布入口仍保持为 GPUI（`.github/workflows/windows.yml` 维持不变），待里程碑 D 全量窗口迁完后统一切换。

- **目录结构调整**：前端与 Tauri 宿主已从原规划的 `apps/desktop/` 直接收口到仓库根目录，采用 `pnpm` 管理依赖与构建：
  - 根目录：`package.json`、`pnpm-lock.yaml`、`vite.config.ts`、`src/`（Vue 3 主窗口与 IPC 层）、`tests/`（前端单元测试）
  - 独立 Rust 工作区：`src-tauri/`（Tauri 2 宿主与平台生命周期），产物为 `kitemail-tauri-preview.exe`
  - 共享 Rust 核心：`crates/mail-core/`、`crates/windows-mail-reader/`（位于根 Cargo 工作区）
- **已完成核心能力**：
  - **里程碑 A**：独立 Tauri 2 窗口、`kitemail-windows-mail-reader` 提取与同窗 WebView2 HWND 挂载验证。
  - **里程碑 B**：`--demo` 模式 10,000 封示例收件箱、三栏分栏、主题切换、虚拟列表分页与搜索。
  - **里程碑 C**：`Live` 模式双层目录隔离校验、只读缓存首屏、后台 `MailWorkerHandle` 单次回复分页查询（`MailWorkerSender::query_page`）、正文在途去重与超时重试、自动已读规则、PBKDF2 应用锁与冷却退避、系统托盘、单实例互斥量与空闲锁定。
  - **三态主题偏好对齐**：`MainPreferencesDto.darkMode` 为 `boolean | null`（`null` 表示跟随系统，前端按 `prefers-color-scheme` 渲染且不覆写 `null`）。

### 9.2 里程碑 C 与 Wave 1 新增 IPC 契约

在原第 4 节首批 6 个命令之外，主窗口已按阶段补充注册以下冻结 IPC 命令（所有带参命令均通过 `{ input }` 包装，结构体字段统一 `camelCase`，锁定时敏感命令统一返回 `IpcError.code = "LOCKED"`）：

| 阶段 | 命令 | 输入与输出 | 说明 |
| --- | --- | --- | --- |
| C | `unlock_app` | `{ passcode }` → `{ unlocked, locked, error?, retryAfterSecs? }` | PBKDF2 解锁校验与冷却退避 |
| C | `lock_app` | 无参 → `{ locked }` | 手动锁屏并隐藏/销毁原生阅读器 |
| C | `set_message_seen` | `{ accountId, messageId, seen }` → `{ accountId, messageId, unread, stateVersion }` | 标记单封邮件已读/未读 |
| C | `set_message_flagged` | `{ accountId, messageId, flagged }` → `{ accountId, messageId, flagged, stateVersion }` | 切换星标状态 |
| C | `quit_app` | 无参 → `void` | 托盘或菜单显式退出 |
| Wave 1 | `allow_remote_images` | `{ accountId, messageId, trustSender }` → `{ allowed, senderTrusted, stateVersion }` | 本次放行远程图片；`trustSender` 为 `true` 时持久化发件人到 `allowed_remote_senders` 并重载阅读器 |
| Wave 1 | `load_older_messages` | `{ view, limit }` → `{ accepted, hasMoreRemote, stateVersion }` | 触发当前文件夹远端历史邮件回填，更新 `hasMoreRemote` 并推送 `main://mail-changed` |
| Wave 1 | `archive_message` | `{ accountId, messageId }` → `{ accountId, messageId, rfcMessageId, originalFolderPath, targetFolderPath, stateVersion }` | 归档邮件并返回撤销所需元数据 |
| Wave 1 | `delete_message` | `{ accountId, messageId }` → `{ accountId, messageId, rfcMessageId, originalFolderPath, targetFolderPath, stateVersion }` | 删除邮件（移入已删除/废纸篓） |
| Wave 1 | `mark_message_spam` | `{ accountId, messageId }` → `{ accountId, messageId, rfcMessageId, originalFolderPath, targetFolderPath, stateVersion }` | 标记为垃圾邮件 |
| Wave 1 | `move_message` | `{ accountId, messageId, targetFolderPath }` → `{ accountId, messageId, rfcMessageId, originalFolderPath, targetFolderPath, stateVersion }` | 移动邮件到指定目标文件夹 |
| Wave 1 | `undo_message_move` | `{ accountId, messageId, rfcMessageId, currentFolderPath, originalFolderPath }` → `{ accepted, stateVersion }` | 撤销最近一次归档/删除/标垃圾/移动操作 |

补充字段说明：
- `OpenMessageOutput` 新增可选字段 `remoteImagesBlocked?: boolean`、`rfcMessageId?: string`、`folderPath?: string`，用于驱动阅读区远程图片隐私提示条与撤销操作上下文。
- 多封会话正文在后端调用 `kitemail_windows_mail_reader::build_thread_document` 渲染，并通过 `ReaderEvent::ThreadAction` 转发折叠展开、远程图放行、星标与已读动作。

### 9.3 开发脚本、同机基准与 Windows CI 用法

1. **多 Worktree 动态端口开发脚本（`scripts/dev-tauri.mjs`）**
   - 用途：避免多 worktree 并行开发时默认开发端口冲突，自动在 `127.0.0.1` 探测空闲回环端口并注入 `KITEMAIL_DEV_PORT` 与 Tauri `--config '{"build":{"devUrl":"http://127.0.0.1:<port>"}}'`，端口被抢占时自动切换端口重试。
   - 常用命令：
     ```powershell
     pnpm dev:wt --demo
     pnpm dev:wt --port 1432 --demo
     pnpm dev:wt --max-retries 8
     ```
2. **同机 `--demo` 性能对比脚本（`scripts/benchmark-demo.ps1`）**
   - 用途：落实第 7 节 B.2 要求，使用临时隔离数据目录分别以 `--demo` 启动 GPUI（默认 `target/x86_64-pc-windows-msvc/release/kitemail.exe`）与 Tauri+Vue（默认 `src-tauri/target/x86_64-pc-windows-msvc/release/kitemail-tauri-preview.exe`），等待主窗口句柄就绪（`MainWindowHandle -ne 0`）后测量冷启动耗时，并在 `-SampleSeconds` 秒内采样主进程及子进程树（含 WebView2）的工作集与私有内存，最后优雅关闭并输出 Markdown 对比表与 JSON 结果；任一可执行文件缺失时会给出提示与构建命令而不崩溃。
   - 常用命令：
     ```powershell
     pwsh -File .\scripts\benchmark-demo.ps1
     pwsh -File .\scripts\benchmark-demo.ps1 -SampleSeconds 8 -JsonOutPath .\temp\bench-demo.json
     pwsh -File .\scripts\benchmark-demo.ps1 -GpuiExe .\path\to\kitemail.exe -TauriExe .\path\to\kitemail-tauri-preview.exe
     ```
3. **Tauri + Vue Windows CI 工作流（`.github/workflows/tauri-windows.yml`）**
   - 与 GPUI 工作流（`.github/workflows/windows.yml`）并行存在、互不修改。
   - 在 `push`（`main`）、`pull_request`（命中 `src/**`、`src-tauri/**`、`crates/mail-core/**`、`crates/windows-mail-reader/**`、`package.json`、`pnpm-lock.yaml`、`vite.config.ts`、`.github/workflows/tauri-windows.yml`）以及 `workflow_dispatch` 时于 `windows-latest` 执行：
     - `pnpm install --frozen-lockfile` → `pnpm build` → `pnpm test`
     - `cargo fmt --manifest-path src-tauri/Cargo.toml -- --check`
     - `cargo test --manifest-path src-tauri/Cargo.toml --target x86_64-pc-windows-msvc --locked`
     - `cargo check --workspace --target x86_64-pc-windows-msvc --locked`
     - `pnpm tauri build --target x86_64-pc-windows-msvc --no-bundle`


## 10. Wave 2 / Wave 3 交付汇总与 P4 正式发行切换清单

### 10.1 Wave 2 与 Wave 3 新增功能与 IPC 全景

在 Wave 1 基础之上，Wave 2（P1）与 Wave 3（P2/P3/P4 准备）已将里程碑 D 规划的其余主窗口、弹窗与工具全量迁移至 Tauri 2 + Vue 3（所有新增有参命令继续遵循 `{ input }` 外层包装、`camelCase` DTO、锁态返回 `IpcError.code = "LOCKED"` 及凭据/口令脱敏）：

1. **Wave 2：写信、草稿、发件箱撤销与多账号引导（14 个命令 + 3 个事件）**
   - 写信与发件箱（`src-tauri/src/compose.rs`、`src/compose/`）：`list_drafts`、`save_draft`（支持 1.5s 防抖静默自动保存）、`delete_draft`、`send_mail`（含即时入队可撤销窗口与定时发送）、`undo_send_mail`、`list_outbox`、`retry_outbox_item`、`delete_outbox_item`，配套事件 `main://outbox-updated`、`main://draft-saved`，以及附件关键词忘带提醒检测。
   - 多账号管理与首次欢迎页（`src-tauri/src/accounts.rs`、`src/accounts/`）：`list_accounts`（含 QQ / 163 / Gmail / Outlook / iCloud / 通用预设，只暴露 `hasPassword` 不回传明文密码）、`verify_account`、`save_account`、`delete_account`、`set_account_enabled`、`start_microsoft_oauth`，配套事件 `main://account-event`。
2. **Wave 3：附件图库、独立读信打印、通讯录、完整偏好与命令面板（17 个命令）**
   - 附件、图库、打印与独立读信（`src-tauri/src/attachments.rs`、`src/reader-tools/`）：`list_cached_attachments`、`fetch_attachment_preview`（10MB 上限与 base64 dataUrl）、`save_attachment`（防路径穿越校验）、`save_all_attachments`、`get_message_print_html`（严格转义与脚本清理 + `@media print` 样式）、`open_message_window`（搭配 `StandaloneMessageModal.vue` / `AttachmentDrawer.vue` / `AttachmentGalleryModal.vue`）。
   - 通讯录（`src-tauri/src/contacts.rs`、`src/contacts/`）：`list_contacts`、`save_contact`、`delete_contact`、`import_contacts`、`export_contacts`（复用 `kitemail_mail_core::contacts_io` 支持 vCard 3.0 与 CSV，一键直连写信）。
   - 完整偏好设置、安全锁、远图白名单、缓存与更新（`src-tauri/src/settings.rs`、`src/preferences/`）：`get_full_preferences`、`update_full_preferences`、`set_app_lock_passcode`（PBKDF2 校验与脱敏）、`manage_trusted_senders`、`clear_body_caches`、`check_app_update`（复用 `kitemail_mail_core::update::check`）。
   - 命令面板与单键快捷键（`src/command-palette/`）：`Ctrl+K` 命令面板、`?` 快捷键帮助、`/` 搜索及受 `singleKeyShortcuts` 与 IME / 输入焦 / 弹窗守卫控制的单键操作（`c/r/a/f/e/s/u/j/k/#`）。

### 10.2 P4 正式发行切换清单与当前切换状态

正式发行入口已切换为 Tauri 2 + Vue 3（仍保留 GPUI 源码在 `crates/app` 供工作区编译校验，待首个正式版本真机及发布 Tag 验证后视需要再行下线）：

1. **产物标识、单实例与正式资料目录对齐（已完成配置与编译验证）**
   - [x] `src-tauri/tauri.conf.json`、`src-tauri/Cargo.toml`、`src-tauri/Cargo.lock`：`productName` 与二进制名已改为 `kitemail`（产出 `src-tauri/target/x86_64-pc-windows-msvc/release/kitemail.exe`），应用标识改为 `com.kitemail.desktop`，主窗口标题改为 `KiteMail`，并注入 `KITEMAIL_VERSION`。
   - [x] `src-tauri/src/single_instance.rs` 与 `src-tauri/src/tray.rs`：单实例互斥量统一切换为 `Local\KiteMail_SingleInstance_Mutex`（与原版保持互斥），托盘窗口类改为 `KiteMailTrayWindowClass`。
   - [x] `src-tauri/src/preferences.rs`：正式 `Live` 模式改回沿用 `kitemail_mail_core::config::resolve_data_dir`（`KITEMAIL_DATA_DIR` > `KITEMAIL_PREVIEW_DATA_DIR` > 便携 `<exe_dir>\data` > 安装 `%LOCALAPPDATA%\KiteMail`），直接无缝读取原版邮件与账号数据；`--demo` 模式继续隔离在预览目录，不触碰正式资料。
2. **安装程序、便携包与正式发布工作流切换（已完成配置切换）**
   - [x] 更新 [NSIS 安装脚本](../.github/nsis/kitemail.nsi) 默认 `BUILD_DIR` 指向 `..\..\src-tauri\target\x86_64-pc-windows-msvc\release`，沿用 `/P` `/UPDATE` `/R` `/PID=`、注册表 `Software\KiteMail` 与 `Local\KiteMail.Setup` 协议。
   - [x] 更新 [Windows 正式发布工作流](../.github/workflows/windows.yml)：接入 `pnpm build`、`pnpm test`、`src-tauri` 测试、`pnpm tauri build --target x86_64-pc-windows-msvc --no-bundle`、NSIS 安装包、便携 ZIP（含 `portable.flag`）与 `latest.json` 生成（`windows-x86_64` / `windows-x86_64-portable`）。
   - [x] 辅助工作流 [tauri-windows.yml](../.github/workflows/tauri-windows.yml) 调整为 `workflow_dispatch` 手动触发，避免与主发布工作流重复运行。
3. **发版前仍需实机验证项**
   - [ ] 使用老版本 GPUI 真实数据副本（含 `accounts.toml`、`preferences.toml`、Windows Credential Manager 凭据、自定义 `mail_data_dir`、草稿箱与待发队列、应用锁 `passcode`）实机升级启动 Tauri 版 `kitemail.exe`，并验证回滚兼容性。
   - [ ] 在 Windows 桌面实机验证 WebView2 遮挡隐藏、DPI 缩放、托盘关闭/唤醒、浏览器 OAuth 回调及首个 `v*` Release Tag 的 `makensis` + `latest.json` 在线更新全链路。
