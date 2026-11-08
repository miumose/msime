//! 词库在线更新在精简版里已移除。这里只保留宿主内部还会走到的查询接口：没有下载的代次，
//! 一切查询都回答「没有更新」，设置宿主请求更新时报告不可用。

use crate::HostOptions;
use msime_client_core::edition::Edition;
use msime_client_core::resources::ResourceSet;
use serde_json::Value;
use std::path::{Path, PathBuf};

/// 下载代次的租约。精简版不会选中任何代次，所以它永远不会被建立。
pub(crate) struct GenerationLease;

/// 更新记录里选中的代次。精简版不会选中任何代次。
pub(crate) struct Selection {
    pub(crate) desktop: ResourceSet,
    pub(crate) languages: Option<ResourceSet>,
}

pub(crate) fn request(
    _options: &HostOptions,
    _command: &str,
    _enabled: Option<bool>,
) -> Result<Value, String> {
    Err("dictionary_update_unavailable".into())
}

pub(crate) fn selected(_root: &Path, _baseline: &ResourceSet) -> Option<Selection> {
    None
}

pub(crate) fn directory(_root: &Path, _set: &ResourceSet) -> Result<PathBuf, std::io::Error> {
    Err(std::io::Error::from(std::io::ErrorKind::NotFound))
}

pub(crate) fn generation_file(
    _directory: &Path,
    _set: &ResourceSet,
    _name: &str,
) -> Option<PathBuf> {
    None
}

pub(crate) fn bundled_resources(_root: &Path, _baseline: &ResourceSet) -> Option<PathBuf> {
    None
}

pub(crate) fn lease(_root: &Path, _resources: &Path) -> Option<GenerationLease> {
    None
}

pub(crate) fn lease_read_only(_root: &Path, _resources: &Path) -> Option<GenerationLease> {
    None
}

pub(crate) fn record_loaded(_root: &Path, _resources: &Path) -> Option<()> {
    None
}

pub(crate) fn selected_resources(
    _state: &Path,
    _specification: &ResourceSet,
) -> Option<(PathBuf, ResourceSet)> {
    None
}

pub(crate) fn languages(_state: &Path, _edition: &Edition) -> Option<PathBuf> {
    None
}

pub(crate) fn verify(_state: &Path, _directory: &Path, _set: &ResourceSet) -> Option<()> {
    None
}
