//! 按键音、上屏音、打字特效和背景音乐在精简版里连同插件音效包一起移除。
//!
//! 生命周期核心仍会调用这里的接口，但所有调用都是空操作：不出声、不画特效、不播音乐。

use msime_client_core::preferences::PluginPreferences;
use std::time::Instant;

/// 按键类别，与 `msime_client_key_sound` 的编号一致。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum KeyClass {
    Default = 0,
    Space = 1,
    Enter = 2,
    Backspace = 3,
}

impl KeyClass {
    pub(crate) const ALL: [Self; 4] = [Self::Default, Self::Space, Self::Enter, Self::Backspace];

    pub(crate) fn from_code(code: u32) -> Option<Self> {
        Self::ALL.into_iter().find(|class| *class as u32 == code)
    }

    pub(crate) fn name(self) -> &'static str {
        match self {
            Self::Default => "default",
            Self::Space => "space",
            Self::Enter => "enter",
            Self::Backspace => "backspace",
        }
    }
}

/// 全屏前台时静音。
pub(crate) const FLAG_MUTED: u32 = 1 << 4;
/// 按住不放的自动重复。
pub(crate) const FLAG_AUTO_REPEAT: u32 = 1 << 8;
/// 上屏事件。
pub(crate) const EVENT_COMMIT: u32 = 0x10;

/// 音效设置。精简版没有音效包，保留这个类型只为让生命周期接口不变。
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct SoundSettings;

impl SoundSettings {
    pub(crate) fn new(_preferences: &PluginPreferences) -> Self {
        Self
    }
}

/// 一个会话的音效状态。没有播放器，所有方法都是空操作。
#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct SessionSound;

impl SessionSound {
    pub(crate) fn new(_settings: SoundSettings) -> Self {
        Self
    }

    pub(crate) fn update(&mut self, _settings: SoundSettings) {}

    pub(crate) fn restamp(&mut self) {}
}

pub(crate) fn key(_sound: &SessionSound, _class: KeyClass) {}

pub(crate) fn commit(_sound: &SessionSound) {}

pub(crate) fn music_active(_sound: &SessionSound, _playing: bool) {}

pub(crate) fn typing_effect(_sound: &SessionSound, _kind: u32, _at: Instant) -> u32 {
    0
}
