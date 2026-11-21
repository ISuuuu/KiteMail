//! 三栏与全局视图模块：左栏 sidebar、中栏 message_list、右栏阅读态 reader、
//! 写信/回复面板 compose、账号管理 accounts、首次启动欢迎向导 welcome、
//! 头像 avatar，以及阶段 3 接入的全局视图 / 浮层组件——附件总览 gallery、
//! 联系人 contacts（主内容路由视图），偏好设置 preferences、
//! 命令面板 command_palette（全局浮层）。

pub(crate) mod accounts;
pub(crate) mod avatar;
pub(crate) mod command_palette;
pub(crate) mod compose;
pub(crate) mod contacts;
pub(crate) mod gallery;
pub(crate) mod message_list;
pub(crate) mod preferences;
pub(crate) mod reader;
pub(crate) mod sidebar;
pub(crate) mod welcome;
pub(crate) mod window_controls;
