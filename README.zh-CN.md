<p align="center">
  <img src="data/repo/logo.png" width="120" alt="KiteMail logo">
</p>

<h1 align="center">KiteMail</h1>

<p align="center">
  基于 Rust、Tauri 2、Vue 3 与 WebView2 构建的原生 Windows 桌面邮件客户端。
</p>

<p align="center">
  <a href="https://github.com/ISuuuu/KiteMail/releases">Releases 下载</a> ·
  <a href="CHANGELOG.md">更新日志</a> ·
  <a href="https://github.com/ISuuuu/KiteMail/issues">问题反馈</a> ·
  <a href="README.md">English</a>
  <br>
  <img alt="Platform: Windows x64" src="https://img.shields.io/badge/platform-Windows%20x64-0078D4">
  <img alt="License: AGPL-3.0-or-later" src="https://img.shields.io/badge/license-AGPL--3.0--or--later-blue">
</p>

---

**KiteMail** 是一个注重性能、隐私与桌面体验的原生 Windows 电子邮件客户端，基于 **Rust**、**Tauri 2**、**Vue 3** 与 **Microsoft Edge WebView2** 构建（同时在 [`crates/app`](crates/app) 保留了早期 GPUI 桌面端实现），不收集任何遥测或统计数据。

> [!NOTE]
> **项目独立说明**：**KiteMail** 最初派生自 Linux GNOME 邮件客户端 [hyprlab/hylki](https://github.com/hyprlab/hylki)，现已作为独立原生 Windows 邮件客户端持续演进与维护。源码分布在 [`src/`](src/)、[`src-tauri/`](src-tauri/) 与 [`crates/`](crates/)，采用独立的软件架构与 `0.x` 版本体系，与上游 Linux `1.x` 版本相互独立。

## 核心特性

- **多账号与主流协议**：支持多账号 IMAP / SMTP（含 IMAP IDLE 实时推送与定时同步）及 Microsoft 365 / Outlook OAuth2（Microsoft Graph）；内置 QQ 邮箱、网易 163/126、腾讯企业邮、阿里企业邮、Gmail、iCloud、Yahoo、Fastmail、Proton Mail Bridge 等预设配置。
- **原生 GPUI 界面与高响应交互**：基于 GPUI 构建的三栏布局（可折叠侧边栏、邮件列表、阅读器），支持阅读窗格位置切换（右侧 / 下方 / 关闭）、专注模式、半屏自适应窄轨、浅色/深色/跟随系统主题、单实例窗口唤醒与系统托盘驻留。
- **WebView2 安全阅读**：使用 WebView2 渲染 HTML 邮件正文，默认拦截远程图片并支持按需加载，剥离外部请求头与 Referrer，净化外链后通过系统浏览器打开；支持基于 RFC 头的会话聚合、引用折叠、内联图片、独立阅读窗口、打印预览以及 `Authentication-Results` 发件验证展示。
- **高效写信与日常管理**：支持富文本 HTML、Markdown 与纯文本等四种编辑模式，支持 HTML 签名清洗与导入、内联回复、独立写信窗口、草稿自动保存、定时发送与重启可恢复的撤销发送；内置全文搜索、多条件筛选、附件总览、通讯录（VCF/CSV 导入导出）、命令面板（`Ctrl+Shift+K`）及单键快捷键模式。
- **分账号分库存储与便携隔离**：采用全局元数据库（`global.db`）与分账号独立目录（轻量索引 `mail.db`、懒加载正文库 `content.db` 及分桶附件目录），支持自定义邮件目录与迁移/复用，并通过 `portable.flag` 提供便携版完整隔离。
- **凭据安全与自动更新**：密码与 OAuth 令牌加密保存于 Windows 凭据管理器；内置应用内检查更新，支持 SHA-256 校验、系统代理（WinHTTP / PAC / 环境变量）与系统根证书、断点续传及 Beta 预发布渠道隔离。

## 下载与安装

前往 [GitHub Releases](https://github.com/ISuuuu/KiteMail/releases) 下载最新 Windows x64 构建产物：

- **安装版（`KiteMail_<version>_x64-setup.exe`）**：NSIS 安装包，默认安装至 `%LOCALAPPDATA%\KiteMail`，配置与本地数据存放于 `%LOCALAPPDATA%\KiteMail\`，支持应用内静默更新。
- **便携版（`KiteMail_v<version>_portable.zip`）**：解压即用，内置 `portable.flag` 标记，配置与全局索引写入程序同级 `data\` 目录，支持应用内就地更新与自定义共享邮件目录。

如需在未配置真实账号时快速体验界面，可通过 `--demo` 参数启动演示模式（例如源码下运行 `pnpm dev:wt -- --demo`，或执行 `kitemail.exe --demo`）。

## 从源码构建

### 环境要求

- Windows x64
- [Rust 工具链](https://rustup.rs/)（`x86_64-pc-windows-msvc`）
- [Node.js 24+](https://nodejs.org/) 与 [pnpm 10+](https://pnpm.io/)
- Microsoft Edge WebView2 运行时（Windows 10/11 通常已内置）

### 编译与测试

```pwsh
git clone https://github.com/ISuuuu/KiteMail.git
cd KiteMail

# 安装前端依赖
pnpm install --frozen-lockfile

# 运行前端单元测试、类型检查与 Vite 构建
pnpm test
pnpm build

# 运行 Rust 核心库与 Tauri 桌面后端测试
cargo test -p kitemail-mail-core
cargo test -p kitemail-windows-mail-reader
cargo test --manifest-path src-tauri/Cargo.toml --locked

# 开发模式启动桌面端（自动探测空闲回环 Vite 端口；附加 `-- --demo` 进入演示模式）
pnpm dev:wt
pnpm dev:wt -- --demo

# 构建 Tauri 桌面可执行文件（no-bundle）
pnpm tauri build --target x86_64-pc-windows-msvc --no-bundle

# 传统 GPUI 工作区检查与测试
cargo check --workspace --locked
cargo test --workspace --locked
```

## 仓库结构

- [`src/`](src/)：Vue 3 + TypeScript 前端源码（主窗口、内嵌通讯录与附件图库、阅读器抽屉、写信编辑器与独立设置窗口）
- [`src-tauri/`](src-tauri/)：Tauri 2 Rust 后端宿主（窗口生命周期、IPC 命令与 DTO、Demo/Live 会话、Windows WinRT PDF 首页缩略图渲染）
- [`crates/`](crates/)：共享 Rust 工作区 crate（[`crates/mail-core`](crates/mail-core) 邮件引擎/缓存/同步、[`crates/windows-mail-reader`](crates/windows-mail-reader) 独立 WebView2 阅读器，以及 [`crates/app`](crates/app) 早期 GPUI 桌面端）
- [`tests/`](tests/) 与 [`scripts/`](scripts/)：前端单测与集成校验入口（`tests/run.mjs`）及多 worktree 开发启动脚本（`scripts/dev-tauri.mjs`）
- [`.github/`](.github/)：Windows x64 CI/CD 工作流（`tauri-windows.yml`、`windows.yml`）与 NSIS 安装包脚本
- [`data/`](data/)：应用图标、欢迎页字标、品牌/SVG 图标资源、视觉参考图与贡献者名单
- [`tools/release-notes.sh`](tools/release-notes.sh)：GitHub Release 说明生成脚本
- [`docs/`](docs/)：第三方许可声明、上游致谢名单与安全策略
- [`CHANGELOG.md`](CHANGELOG.md)：Windows 客户端更新日志与早期历史记录

## 参与贡献与反馈

欢迎在 [GitHub Issues](https://github.com/ISuuuu/KiteMail/issues) 提交缺陷反馈、功能建议或 Pull Request。安全相关问题请参考 [docs/SECURITY.md](docs/SECURITY.md)。

## 许可证与致谢

- 本项目遵循 **GNU Affero General Public License v3.0 or later**（[AGPL-3.0-or-later](LICENSE)）开源协议发布。
- 最初派生自 [hyprlab/hylki](https://github.com/hyprlab/hylki)（© 2026 Hyprlab）。应用图标与字标由 [Yiannis Ioannides](https://github.com/yioannides) 绘制，上游贡献者与翻译者名单见 [data/CONTRIBUTORS](data/CONTRIBUTORS)、[data/TRANSLATORS](data/TRANSLATORS) 与 [docs/CREDITS.md](docs/CREDITS.md)。
- 第三方组件与资源许可声明见 [docs/LICENSE.md](docs/LICENSE.md)。
