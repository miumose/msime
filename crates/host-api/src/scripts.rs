//! 脚本插件已从精简版移除。生命周期核心仍保留调用点，这里的集合是空的：不加载、不运行、
//! 不改写上屏，也不向引擎提供插件指令。

use msime_input_runtime::{DispatchFacts, Runtime, Transition, View};
use std::cell::RefCell;
use std::path::PathBuf;
use std::rc::Rc;

/// 一次调用里可以交给脚本的文本。精简版不运行脚本，只需要类型存在。
#[derive(Default)]
pub(crate) struct CallTexts<'a> {
    pub(crate) view: Option<&'a View>,
    pub(crate) facts: DispatchFacts,
}

impl<'a> CallTexts<'a> {
    pub(crate) fn of(
        transition: &'a Transition,
        facts: DispatchFacts,
        _selected: Option<&str>,
    ) -> Self {
        Self {
            view: Some(&transition.view),
            facts,
        }
    }
}

/// 上屏改写请求。精简版不会产生改写。
pub(crate) struct RewriteRequest<'a> {
    pub(crate) origin: u8,
    pub(crate) rewritable: bool,
    pub(crate) local_mode: Option<&'a str>,
    pub(crate) body: &'a str,
    pub(crate) tail: &'a str,
    pub(crate) selected: Option<&'a str>,
}

/// 一个会话的脚本实例集合。这里恒为空。
#[derive(Default)]
pub(crate) struct ScriptSet;

impl ScriptSet {
    pub(crate) fn bind(
        &mut self,
        _root: Option<PathBuf>,
        _builtin_packs: Option<PathBuf>,
        _user_data: &str,
    ) {
    }

    pub(crate) fn focus_in(&mut self) {}

    pub(crate) fn release(&mut self) {}

    pub(crate) fn plugin_commands(&mut self) -> Vec<msime_input_runtime::PluginCommand> {
        Vec::new()
    }

    pub(crate) fn traits_reported<C>(&mut self, _context: C) {}

    pub(crate) fn key_reported(&mut self) {}

    pub(crate) fn next_turn(&mut self) {}

    pub(crate) fn input<C>(
        &mut self,
        _context: C,
        _class: crate::lifecycle::InputClass,
        _local_mode: crate::lifecycle::LocalModeClass,
        _input_char: Option<char>,
    ) {
    }

    pub(crate) fn before_commit<C>(
        &mut self,
        _context: C,
        _request: &RewriteRequest<'_>,
    ) -> Option<String> {
        None
    }

    pub(crate) fn deliver<C>(
        &mut self,
        _event: &crate::lifecycle::LifecycleEvent,
        _context: C,
        _texts: &CallTexts<'_>,
    ) {
    }
}

/// 把脚本集合挂到 runtime 上。精简版不挂。
pub(crate) fn attach(_scripts: &Rc<RefCell<ScriptSet>>, _runtime: &mut Runtime) {}

/// 上屏只能观察时让脚本变换停下。精简版没有脚本变换。
pub(crate) fn observe_only(_runtime: &mut Runtime, _observe_only: bool) {}

/// 进程级开关。精简版永远没有脚本插件。
pub(crate) fn enable(_on: bool) -> bool {
    false
}
