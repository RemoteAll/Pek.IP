//! `Config/Ip.toml`：平台地址等专门配置项。
//!
//! 平台地址固定为默认值；如需更换 RPanelServer，只改 `StoreUrl` 一行即可，无需改代码。
//! 首次加载时若 `Config/Ip.toml` 不存在，自动按内置模板生成。
//!
//! 配置优先级：环境变量 > `Config/Ip.toml` > 默认值。

use std::fs;
use std::path::{Path, PathBuf};

use crate::auth::IpAuthConfig;
use crate::{Error, IpDbManagerConfig, IpLocator, LocatorLoadOptions, Result};

/// 默认 RPanelServer 平台地址（固定；一般无需修改）。
pub const DEFAULT_STORE_URL: &str = "https://p.sc8.fun";

const CONFIG_DIR: &str = "Config";
const CONFIG_FILE: &str = "Ip.toml";
const TEMPLATE: &str = include_str!("../res/Ip.toml.template");

/// Pek.IP 平台配置。
#[derive(Clone, Debug)]
pub struct IpConfig {
    /// RPanelServer 平台地址（固定，一般不改）。
    pub store_url: String,
    /// 平台 Ed25519 公钥（非空 = 强制验签）。
    pub pubkey: String,
    /// 本地数据目录（`data/`）。
    pub data_dir: PathBuf,
    /// 下载缓存目录。
    pub cache_dir: PathBuf,
    /// 客户端状态目录（installId/密钥/License；为后续 auto 模式预留）。
    pub state_dir: PathBuf,
    /// 是否允许自动注册（为后续 auto 模式预留）。
    pub auto_enroll: bool,
    /// 是否允许下载 IPv6 xdb（约 37 MiB）。
    pub download_v6: bool,
    /// 平台 https 自定义根证书（PEM 文件）。
    pub ca_file: Option<PathBuf>,
}

impl Default for IpConfig {
    fn default() -> Self {
        Self {
            store_url: DEFAULT_STORE_URL.to_string(),
            pubkey: String::new(),
            data_dir: PathBuf::from("data"),
            cache_dir: PathBuf::from("Data/IpCache"),
            state_dir: PathBuf::from("Data/IpClient"),
            auto_enroll: true,
            download_v6: false,
            ca_file: None,
        }
    }
}

impl IpConfig {
    /// 读取 `{base}/Config/Ip.toml`；不存在时自动生成模板。
    pub fn load_or_default(base: impl AsRef<Path>) -> Result<Self> {
        let base = base.as_ref();
        let path = base.join(CONFIG_DIR).join(CONFIG_FILE);
        if !path.is_file() {
            if let Some(dir) = path.parent() {
                fs::create_dir_all(dir)?;
            }
            fs::write(&path, TEMPLATE)?;
        }
        let mut cfg = Self::parse_file(&path, base)?;
        cfg.apply_env(base);
        Ok(cfg)
    }

    /// 解析 TOML 文件（相对路径按 `base` 解析）。
    pub fn parse_file(path: &Path, base: &Path) -> Result<Self> {
        let text = fs::read_to_string(path)?;
        let doc = dhrust::config::toml::parse_document(&text)
            .map_err(|e| Error::Argument(format!("{} 解析失败：{e}", path.display())))?;
        let json = dhrust::config::toml::document_to_json(&doc)
            .map_err(|e| Error::Argument(format!("{} 转换失败：{e}", path.display())))?;
        let mut cfg = Self::default();
        let get = |key: &str| {
            json.get(key)
                .and_then(|v| v.as_str())
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .map(str::to_string)
        };
        if let Some(v) = get("StoreUrl") {
            cfg.store_url = v;
        }
        if let Some(v) = get("PubKey") {
            cfg.pubkey = v;
        }
        if let Some(v) = get("DataDir") {
            cfg.data_dir = resolve_path(base, &v);
        } else {
            cfg.data_dir = base.join("data");
        }
        if let Some(v) = get("CacheDir") {
            cfg.cache_dir = resolve_path(base, &v);
        } else {
            cfg.cache_dir = base.join("Data").join("IpCache");
        }
        if let Some(v) = get("StateDir") {
            cfg.state_dir = resolve_path(base, &v);
        } else {
            cfg.state_dir = base.join("Data").join("IpClient");
        }
        if let Some(v) = json.get("AutoEnroll").and_then(|v| v.as_bool()) {
            cfg.auto_enroll = v;
        }
        if let Some(v) = json.get("DownloadV6").and_then(|v| v.as_bool()) {
            cfg.download_v6 = v;
        }
        if let Some(v) = get("CaFile") {
            cfg.ca_file = Some(resolve_path(base, &v));
        }
        cfg.store_url = cfg.store_url.trim().trim_end_matches('/').to_string();
        Ok(cfg)
    }

    /// 环境变量覆盖（便于 CI/临时切换，无需改文件）。
    fn apply_env(&mut self, base: &Path) {
        if let Ok(v) = std::env::var("PEK_IP_STORE_URL")
            && !v.trim().is_empty()
        {
            self.store_url = v.trim().trim_end_matches('/').to_string();
        }
        if let Ok(v) = std::env::var("PEK_IP_PUBKEY")
            && !v.trim().is_empty()
        {
            self.pubkey = v.trim().to_string();
        }
        if let Ok(v) = std::env::var("PEK_IP_DATA_DIR")
            && !v.trim().is_empty()
        {
            self.data_dir = resolve_path(base, v.trim());
        }
        if let Ok(v) = std::env::var("PEK_IP_CACHE_DIR")
            && !v.trim().is_empty()
        {
            self.cache_dir = resolve_path(base, v.trim());
        }
        if let Ok(v) = std::env::var("PEK_IP_STATE_DIR")
            && !v.trim().is_empty()
        {
            self.state_dir = resolve_path(base, v.trim());
        }
        if let Ok(v) = std::env::var("PEK_IP_AUTO_ENROLL") {
            self.auto_enroll =
                !matches!(v.trim().to_ascii_lowercase().as_str(), "0" | "false" | "no");
        }
        if let Ok(v) = std::env::var("PEK_IP_DOWNLOAD_V6") {
            self.download_v6 =
                matches!(v.trim().to_ascii_lowercase().as_str(), "1" | "true" | "yes");
        }
    }

    /// 读取 CA PEM（若配置）。
    pub fn ca_pem(&self) -> Result<Option<Vec<u8>>> {
        match &self.ca_file {
            Some(path) if path.is_file() => Ok(Some(fs::read(path)?)),
            Some(path) => Err(Error::MissingFile(path.clone())),
            None => Ok(None),
        }
    }

    /// 转成管理器配置。
    pub fn to_manager_config(&self) -> Result<IpDbManagerConfig> {
        let ca_pem = self.ca_pem()?;
        let platform_pubkey = {
            let key = self.pubkey.trim();
            if key.is_empty() {
                None
            } else {
                Some(key.to_string())
            }
        };
        Ok(IpDbManagerConfig {
            store_url: self.store_url.clone(),
            pubkey: self.pubkey.clone(),
            ca_pem: ca_pem.clone(),
            cache_dir: Some(self.cache_dir.clone()),
            auth: IpAuthConfig {
                // 零配置：默认自动注册；平台不支持时客户端自动回退公开下载
                enabled: self.auto_enroll,
                state_dir: Some(self.state_dir.clone()),
                platform_pubkey,
                ca_pem,
                ..Default::default()
            },
            ..Default::default()
        })
    }

    /// 转成启动加载选项。
    pub fn to_load_options(&self) -> LocatorLoadOptions {
        LocatorLoadOptions {
            data_dir: self.data_dir.clone(),
            download_v6: self.download_v6,
            ..Default::default()
        }
    }
}

impl IpLocator {
    /// 零配置加载：读取 `Config/Ip.toml`（不存在自动生成）+ 本地 `data/`，缺失按需下载。
    ///
    /// 平台地址固定，默认 `https://p.sc8.fun`；如需更换只改 `Config/Ip.toml` 的 `StoreUrl`。
    ///
    /// ```no_run
    /// # fn main() -> Result<(), pek_ip::Error> {
    /// let locator = pek_ip::IpLocator::load_default()?;
    /// # Ok(()) }
    /// ```
    pub fn load_default() -> Result<Self> {
        let base = dhrust::io::base_dir(&["PEK_IP_BASE"]);
        let cfg = IpConfig::load_or_default(&base)?;
        IpLocator::load_or_download(cfg.to_manager_config()?, cfg.to_load_options())
    }
}

fn resolve_path(base: &Path, value: &str) -> PathBuf {
    let path = Path::new(value);
    if path.is_absolute() {
        path.to_path_buf()
    } else {
        base.join(path)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_store_url_is_fixed() {
        assert_eq!(DEFAULT_STORE_URL, "https://p.sc8.fun");
        let cfg = IpConfig::default();
        assert_eq!(cfg.store_url, DEFAULT_STORE_URL);
        assert!(cfg.auto_enroll);
    }

    #[test]
    fn parses_template_and_resolves_paths() {
        let base = std::env::temp_dir().join(format!("pek-ip-config-{}", std::process::id()));
        let _ = fs::remove_dir_all(&base);
        fs::create_dir_all(base.join("Config")).unwrap();
        fs::write(base.join("Config").join("Ip.toml"), TEMPLATE).unwrap();
        let cfg = IpConfig::parse_file(&base.join("Config").join("Ip.toml"), &base).unwrap();
        assert_eq!(cfg.store_url, DEFAULT_STORE_URL);
        assert_eq!(cfg.data_dir, base.join("data"));
        assert_eq!(cfg.cache_dir, base.join("Data").join("IpCache"));
        assert_eq!(cfg.state_dir, base.join("Data").join("IpClient"));
        let _ = fs::remove_dir_all(&base);
    }
}
