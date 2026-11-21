//! 三栏 UI 模块：左栏 sidebar、中栏 message_list、右栏阅读态 reader、
//! 头像 avatar。写信 / 账号设置本轮仍由 `app.rs` 渲染（迭代 2 再拆）。

pub(crate) mod avatar;
pub(crate) mod command_palette;
pub(crate) mod contacts;
pub(crate) mod gallery;
pub(crate) mod message_list;
pub(crate) mod preferences;
pub(crate) mod reader;
pub(crate) mod sidebar;
