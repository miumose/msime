//! 剪贴板历史已随功能删除；保留入口让偏好的调用方不变。

use super::{PreferencesError, PreferencesStore};
use std::path::Path;

impl PreferencesStore {
    pub fn clear_disabled_clipboard_history_at(&self, _history_path: &Path) -> Result<bool, PreferencesError> {
        Ok(false)
    }
}
