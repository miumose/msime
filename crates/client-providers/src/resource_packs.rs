//! 资源包下载在精简版里只剩按给定前缀顺序尝试；项目镜像列表为空。

pub const PROJECT_MIRRORS: [&str; 0] = [];

pub fn download_prefixes(prefixes: &[String]) -> Vec<&str> {
    prefixes.iter().map(String::as_str).collect()
}
