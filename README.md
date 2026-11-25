<p align="center">
  <img src="data/repo/logo.png" width="120" alt="KiteMail logo">
</p>

<h1 align="center">KiteMail</h1>

<p align="center">
  A native Windows desktop email client built with Rust, GPUI, and WebView2.
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

**KiteMail** is a fast, privacy-focused native email client for Windows, built with Rust, GPUI (`gpui-kit`), and Microsoft Edge WebView2. It sends no telemetry or analytics.

> [!NOTE]
> **Project Independence & Rename**: **KiteMail** (formerly Hylki for Windows) originally started as a fork of the Linux GNOME email client [hyprlab/hylki](https://github.com/hyprlab/hylki) and is now developed and maintained as an independent project. The Windows client lives in [`desktop/`](desktop/) with its own architecture and `0.x` release cycle, independent from the upstream Linux `1.x` series. Package/binary names (`hylki`) and data directories are scheduled to migrate to KiteMail in the next release.

## Features

- **Multi-Account & Protocols**: Manage multiple accounts with IMAP / SMTP (including IMAP IDLE push and scheduled sync) as well as Microsoft 365 / Outlook OAuth2 (Microsoft Graph). Includes built-in presets for QQ Mail, NetEase 163/126, Tencent Exmail, Aliyun Enterprise Mail, Gmail, iCloud, Yahoo, Fastmail, Proton Mail Bridge, and more.
- **Native GPUI Interface**: Responsive three-pane layout (collapsible sidebar, message list, and reader) with configurable reader placement (right, bottom, or hidden), Focus Mode, adaptive narrow rail on half-screen windows, Light/Dark/System themes, single-instance activation, and system tray minimization.
- **Secure WebView2 Reader**: Renders HTML messages inside WebView2 with remote images blocked by default (loadable on demand), external request headers and `Referrer` stripped, and sanitized links opened in your default browser. Supports RFC-header conversation threading, quote folding, inline images, standalone reader windows, print preview, and `Authentication-Results` verification badges.
- **Rich Composing & Productivity**: Four editing modes (including rich HTML, Markdown, and plain text), HTML signature sanitization and import, inline reply, standalone compose windows, auto-saved drafts, scheduled send, and restart-safe Undo Send. Includes full-text search, multi-criteria filtering, an attachment gallery, contacts management (VCF/CSV import & export), Command Palette (`Ctrl+Shift+K`), and optional single-key shortcuts.
- **Per-Account Storage & Portable Mode**: Separates global metadata (`global.db`) from per-account storage (`mail.db` index, lazy-loaded `content.db` bodies, and bucketed attachments). Supports custom mail storage directories with migration/reuse and full portable isolation via `portable.flag`.
- **Security & In-App Updates**: Stores passwords and OAuth tokens in Windows Credential Manager. Built-in updater verifies SHA-256 checksums, honors Windows system proxy settings (WinHTTP / PAC / env) and system root certificates, supports resumable downloads, and separates Beta prereleases from the stable channel.

## Installation

Download the latest Windows x64 release from [GitHub Releases](https://github.com/ISuuuu/KiteMail/releases) (currently packaged under the `Hylki` name prior to the next release):

- **Installer (`Hylki_<version>_x64-setup.exe`)**: NSIS installer targeting `%LOCALAPPDATA%\Hylki` by default. Configuration and local data are stored in `%LOCALAPPDATA%\Hylki\`, with silent in-app updates.
- **Portable (`Hylki_v<version>_portable.zip`)**: Extract and run. Includes `portable.flag` so configuration and global state stay inside a sibling `data\` directory, with in-place updates and optional custom mail storage paths.

To preview the UI without adding a real email account, launch the executable with `--demo`.

## Building from Source

### Prerequisites

- Windows x64
- [Rust toolchain](https://rustup.rs/) (`x86_64-pc-windows-msvc`)
- Microsoft Edge WebView2 Runtime (preinstalled on modern Windows 10/11)

### Commands

```pwsh
git clone https://github.com/ISuuuu/KiteMail.git
cd KiteMail

# Check and run unit tests
cargo check --manifest-path desktop/Cargo.toml --workspace --locked
cargo test --manifest-path desktop/Cargo.toml --workspace --locked

# Run in development mode
cargo run --manifest-path desktop/Cargo.toml -p hylki --bin hylki

# Build release binary
cargo build --manifest-path desktop/Cargo.toml -p hylki --bin hylki --release --target x86_64-pc-windows-msvc --locked
```

## Repository Layout

- [`desktop/`](desktop/): Windows client workspace (`hylki` desktop app and `hylki-mail-core` library)
- [`.github/`](.github/): Windows x64 CI/CD workflow and NSIS installer script
- [`CHANGELOG.md`](CHANGELOG.md): Windows client release history and earlier upstream changelog
- [`src/`](src/), [`docs/`](docs/): Legacy upstream Linux (GTK 4 / libadwaita) source and reference docs

## Contributing & Support

Bug reports, feature suggestions, and pull requests are welcome on [GitHub Issues](https://github.com/ISuuuu/KiteMail/issues). For security vulnerabilities, please see [docs/SECURITY.md](docs/SECURITY.md).

## License & Acknowledgments

- Licensed under the **GNU Affero General Public License v3.0 or later** ([AGPL-3.0-or-later](LICENSE)).
- Originally forked from [hyprlab/hylki](https://github.com/hyprlab/hylki) (© 2026 Hyprlab). App icon and wordmark by [Yiannis Ioannides](https://github.com/yioannides). See [data/CONTRIBUTORS](data/CONTRIBUTORS), [data/TRANSLATORS](data/TRANSLATORS), and [docs/CREDITS.md](docs/CREDITS.md) for upstream contributors and translators.
- Third-party notices and bundled assets are listed in [docs/LICENSE.md](docs/LICENSE.md).
