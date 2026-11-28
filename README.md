<p align="center">
  <img src="data/repo/logo.png" width="120" alt="KiteMail logo">
</p>

<h1 align="center">KiteMail</h1>

<p align="center">
  A fast, privacy-focused native Windows desktop email client built with Rust, Tauri 2, Vue 3, and WebView2.
</p>

<p align="center">
  <a href="https://github.com/ISuuuu/KiteMail/releases">Releases</a> ·
  <a href="CHANGELOG.md">Changelog</a> ·
  <a href="https://github.com/ISuuuu/KiteMail/issues">Issues</a> ·
  <a href="README.zh-CN.md">简体中文</a>
  <br>
  <img alt="Platform: Windows x64" src="https://img.shields.io/badge/platform-Windows%20x64-0078D4">
  <img alt="License: AGPL-3.0-or-later" src="https://img.shields.io/badge/license-AGPL--3.0--or--later-blue">
</p>

---

**KiteMail** is a fast, privacy-focused native email client for Windows built with **Rust**, **Tauri 2**, **Vue 3**, and **Microsoft Edge WebView2** (while preserving the earlier GPUI desktop implementation in [`crates/app`](crates/app)). It sends no telemetry or analytics.

> [!NOTE]
> **Project Independence**: **KiteMail** (originally forked from the Linux GNOME email client [hyprlab/hylki](https://github.com/hyprlab/hylki)) is developed and maintained as an independent native Windows email client. It maintains its own architecture and `0.x` release cycle across [`src/`](src/), [`src-tauri/`](src-tauri/), and [`crates/`](crates/), independent from the upstream Linux `1.x` series.

## Features

- **Multi-Account & Protocols**: Manage multiple accounts with IMAP / SMTP (including IMAP IDLE push and scheduled sync) as well as Microsoft 365 / Outlook OAuth2 (Microsoft Graph). Includes built-in presets for QQ Mail, NetEase 163/126, Tencent Exmail, Aliyun Enterprise Mail, Gmail, iCloud, Yahoo, Fastmail, Proton Mail Bridge, and more.
- **Native GPUI Interface**: Responsive three-pane layout (collapsible sidebar, message list, and reader) with configurable reader placement (right, bottom, or hidden), Focus Mode, adaptive narrow rail on half-screen windows, Light/Dark/System themes, single-instance activation, and system tray minimization.
- **Secure WebView2 Reader**: Renders HTML messages inside WebView2 with remote images blocked by default (loadable on demand), external request headers and `Referrer` stripped, and sanitized links opened in your default browser. Supports RFC-header conversation threading, quote folding, inline images, standalone reader windows, print preview, and `Authentication-Results` verification badges.
- **Rich Composing & Productivity**: Four editing modes (including rich HTML, Markdown, and plain text), HTML signature sanitization and import, inline reply, standalone compose windows, auto-saved drafts, scheduled send, and restart-safe Undo Send. Includes full-text search, multi-criteria filtering, an attachment gallery, contacts management (VCF/CSV import & export), Command Palette (`Ctrl+Shift+K`), and optional single-key shortcuts.
- **Per-Account Storage & Portable Mode**: Separates global metadata (`global.db`) from per-account storage (`mail.db` index, lazy-loaded `content.db` bodies, and bucketed attachments). Supports custom mail storage directories with migration/reuse and full portable isolation via `portable.flag`.
- **Security & In-App Updates**: Stores passwords and OAuth tokens in Windows Credential Manager. Built-in updater verifies SHA-256 checksums, honors Windows system proxy settings (WinHTTP / PAC / env) and system root certificates, supports resumable downloads, and separates Beta prereleases from the stable channel.

## Installation

Download the latest Windows x64 release from [GitHub Releases](https://github.com/ISuuuu/KiteMail/releases):

- **Installer (`KiteMail_<version>_x64-setup.exe`)**: NSIS installer targeting `%LOCALAPPDATA%\KiteMail` by default. Configuration and local data are stored in `%LOCALAPPDATA%\KiteMail\`, with silent in-app updates.
- **Portable (`KiteMail_v<version>_portable.zip`)**: Extract and run. Includes `portable.flag` so configuration and global state stay inside a sibling `data\` directory, with in-place updates and optional custom mail storage paths.

To preview the UI without adding a real email account, pass `--demo` (e.g., `pnpm dev:wt -- --demo` from source or `kitemail.exe --demo`).

## Building from Source

### Prerequisites

- Windows x64
- [Rust toolchain](https://rustup.rs/) (`x86_64-pc-windows-msvc`)
- [Node.js 24+](https://nodejs.org/) and [pnpm 10+](https://pnpm.io/)
- Microsoft Edge WebView2 Runtime (preinstalled on modern Windows 10/11)

### Commands

```pwsh
git clone https://github.com/ISuuuu/KiteMail.git
cd KiteMail

# Install frontend dependencies
pnpm install --frozen-lockfile

# Run frontend unit tests and typecheck/build
pnpm test
pnpm build

# Run Rust tests (core crates and Tauri desktop backend)
cargo test -p kitemail-mail-core
cargo test -p kitemail-windows-mail-reader
cargo test --manifest-path src-tauri/Cargo.toml --locked

# Launch Tauri desktop app in development mode (auto-picks free loopback Vite port; add `-- --demo` for demo mode)
pnpm dev:wt
pnpm dev:wt -- --demo

# Build Tauri executable (no-bundle)
pnpm tauri build --target x86_64-pc-windows-msvc --no-bundle

# Legacy GPUI workspace checks (root Cargo workspace)
cargo check --workspace --locked
cargo test --workspace --locked
```

## Repository Layout

- [`src/`](src/): Vue 3 + TypeScript frontend for the main window, embedded workspace views, reader drawer, compose editor, and standalone settings window
- [`src-tauri/`](src-tauri/): Tauri 2 Rust backend, window lifecycle, IPC commands/DTOs, demo/live mail session, and Windows WinRT PDF thumbnail rendering
- [`crates/`](crates/): Shared Rust workspace crates ([`crates/mail-core`](crates/mail-core) mail engine/cache/sync, [`crates/windows-mail-reader`](crates/windows-mail-reader) isolated WebView2 thread reader, and [`crates/app`](crates/app) legacy GPUI desktop UI)
- [`tests/`](tests/) & [`scripts/`](scripts/): Frontend unit/integration test runner (`tests/run.mjs`) and worktree-safe dev launcher (`scripts/dev-tauri.mjs`)
- [`.github/`](.github/): Windows x64 CI/CD workflows (`tauri-windows.yml`, `windows.yml`) and NSIS installer script
- [`data/`](data/): App icons, welcome wordmarks, brand/SVG assets, reference screenshots, and contributor lists
- [`tools/release-notes.sh`](tools/release-notes.sh): Release notes generator for GitHub Releases
- [`docs/`](docs/): Licensing notices, upstream credits, and security policy
- [`CHANGELOG.md`](CHANGELOG.md): Windows client release history and earlier upstream changelog

## Contributing & Support

Bug reports, feature suggestions, and pull requests are welcome on [GitHub Issues](https://github.com/ISuuuu/KiteMail/issues). For security vulnerabilities, please see [docs/SECURITY.md](docs/SECURITY.md).

## License & Acknowledgments

- Licensed under the **GNU Affero General Public License v3.0 or later** ([AGPL-3.0-or-later](LICENSE)).
- Originally forked from [hyprlab/hylki](https://github.com/hyprlab/hylki) (© 2026 Hyprlab). App icon and wordmark by [Yiannis Ioannides](https://github.com/yioannides). See [data/CONTRIBUTORS](data/CONTRIBUTORS), [data/TRANSLATORS](data/TRANSLATORS), and [docs/CREDITS.md](docs/CREDITS.md) for upstream contributors and translators.
- Third-party notices and bundled assets are listed in [docs/LICENSE.md](docs/LICENSE.md).
