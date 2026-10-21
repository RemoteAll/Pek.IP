//! 按需下载 IP 数据包（消费 Pek.RPanlServer 组件源）。
//!
//! 协议与 `Pek.RCode` 的 `driver_pack` 完全一致：
//!
//! - 目录：`{store}/components/catalog.json`（同址 `.sig` 为 base64 Ed25519 签名）；
//! - 条目：`id / version / target / sha256 / url`；
//! - 数据包：zip（平台上传时计算 SHA-256）；
//! - 本地缓存：`{cache}/{id}/{version}/`，解压后写 `.pek-ip.json` 标记；
//! - `store_url` 为空 = 仅本地缓存模式（离线部署/测试）。
//!
//! 安全：
//!
//! - 配置 `pubkey` 后**强制验签**（此时允许内网 http 组件源）；为空时仅允许 https 或回环 http；
//! - 包 SHA-256 强制校验；
//! - 解压使用 `dhrust::zip::extract_zip`（防目录穿越）；
//! - `access_token` 以 `Authorization: Bearer` 发送，供平台后续接入“按需授权/付费下载”；
//! - 启用 [`IpAuthConfig`] 时：首次下载自动注册/获取 License，每次下载申请 5 分钟票据；
//!   旧平台没有 `/api/ipdb/*` 时自动回退公开下载，授权接口 403/401 不回退。

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, mpsc};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use serde_json::{Value as Json, json};

use dhrust::net::http_client::{self, HttpClientOptions};

use crate::auth::{AuthClient, AuthRuntime, IpAuthConfig};
use crate::xdb::XdbReader;
use crate::{Error, IpLocator, Result};

#[cfg(feature = "mmdb")]
use crate::mmdb::MmdbReader;

/// 组件分类（上传到 Pek.RPanlServer 时填写）。
pub const IP_DATABASE_CATEGORY: &str = "ip-database";
/// ip2region IPv4 数据集 id。
pub const DATASET_IP2REGION_V4: &str = "ip2region-v4";
/// ip2region IPv6 数据集 id。
pub const DATASET_IP2REGION_V6: &str = "ip2region-v6";
/// GeoLite2/DB-IP 城市库数据集 id。
pub const DATASET_GEOLITE2_CITY: &str = "geolite2-city";
/// GeoLite2 ASN 库数据集 id。
pub const DATASET_GEOLITE2_ASN: &str = "geolite2-asn";
/// DB-IP 城市库（替代 MaxMind）数据集 id。
pub const DATASET_DBIP_CITY: &str = "dbip-city";

const CATALOG_PATH: &str = "/components/catalog.json";
const DEFAULT_CACHE_DIR: &str = "pek-ip-data";
const MARKER_FILE: &str = ".pek-ip.json";
const MAX_CATALOG_BYTES: usize = 1024 * 1024;
const MAX_SIGNATURE_BYTES: usize = 8 * 1024;
const DEFAULT_PACKAGE_LIMIT: u64 = 512 * 1024 * 1024;

/// 数据管理器配置。
#[derive(Clone, Debug)]
pub struct IpDbManagerConfig {
    /// 平台组件源地址（如 `https://p.sc8.fun`；也可直接给完整 `.../components/catalog.json`）。
    /// 为空 = **仅本地缓存模式**。
    pub store_url: String,
    /// 平台 Ed25519 公钥（64 位 hex，或 44 字节 SPKI DER base64）。非空强制验签。
    pub pubkey: String,
    /// 组件源 https 自定义根证书（PEM，可含多张；自签/内网 CA）。
    pub ca_pem: Option<Vec<u8>>,
    /// 本地缓存目录；缺省为系统临时目录下 `pek-ip-data`。
    pub cache_dir: Option<PathBuf>,
    /// 目录/包请求超时；缺省 60 秒。
    pub timeout: Option<Duration>,
    /// 平台后续“按需授权/付费下载”的访问令牌（`Authorization: Bearer`）。
    pub access_token: Option<String>,
    /// 每个数据集保留的版本数（默认 1；更新后删除旧版本）。
    pub keep_versions: usize,
    /// 单包体积上限（默认 512 MiB）。
    pub package_limit: u64,
    /// 目标平台（缺省按当前平台识别；IP 数据包通常使用 `any`）。
    pub target: Option<String>,
    /// 下载授权（零配置自动注册 + Ed25519 票据；默认关闭，`Config/Ip.toml` 的
    /// `AutoEnroll=true` 时启用）。
    pub auth: IpAuthConfig,
}

impl Default for IpDbManagerConfig {
    fn default() -> Self {
        Self {
            store_url: String::new(),
            pubkey: String::new(),
            ca_pem: None,
            cache_dir: None,
            timeout: None,
            access_token: None,
            keep_versions: 1,
            package_limit: DEFAULT_PACKAGE_LIMIT,
            target: None,
            auth: IpAuthConfig::default(),
        }
    }
}

/// 启动加载选项：优先本地 `data/`，缺失时按需从组件源下载。
///
/// 默认：加载 IPv4 + IPv6（v6 若本地存在） + 国外城市/ASN；
/// 本地缺失时下载 v4/城市/ASN；**v6 默认不下载**（约 37 MiB），需要时显式 `download_v6 = true`。
#[derive(Clone, Debug)]
pub struct LocatorLoadOptions {
    /// 本地数据目录（默认 `data`；相对路径按进程当前目录）。
    pub data_dir: PathBuf,
    /// 加载 IPv4 xdb（默认 true）。
    pub load_v4: bool,
    /// 加载 IPv6 xdb（默认 true；仅本地存在时加载）。
    pub load_v6: bool,
    /// 本地没有 IPv6 xdb 时是否下载（默认 false）。
    pub download_v6: bool,
    /// 加载国外城市/国家 MMDB（默认 true）。
    pub load_world: bool,
    /// 加载国外 ASN/ISP MMDB（默认 true）。
    pub load_asn: bool,
    /// 本地缺失时是否从组件源下载（默认 true）。
    pub download_missing: bool,
    /// 允许部分数据缺失/下载失败（默认 true；启动时尽力而为）。
    pub allow_partial: bool,
}

impl Default for LocatorLoadOptions {
    fn default() -> Self {
        Self {
            data_dir: PathBuf::from("data"),
            load_v4: true,
            load_v6: true,
            download_v6: false,
            load_world: true,
            load_asn: true,
            download_missing: true,
            allow_partial: true,
        }
    }
}

/// 组件源目录条目。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ComponentEntry {
    /// 组件 id（如 `ip2region-v4`）
    pub id: String,
    /// 显示名
    pub name: String,
    /// 分类
    pub category: String,
    /// 版本号
    pub version: String,
    /// 目标平台（数据包通常为 `any`）
    pub target: String,
    /// 包 SHA-256（小写 hex）
    pub sha256: String,
    /// 包下载地址（绝对）
    pub url: String,
    /// 包大小（KB，可能为 0）
    pub size_kb: i64,
    /// 说明
    pub notes: String,
}

impl ComponentEntry {
    fn from_json(item: &Json) -> std::result::Result<Self, String> {
        let field = |key: &str| {
            item.get(key)
                .and_then(|v| v.as_str())
                .map(str::trim)
                .unwrap_or("")
                .to_string()
        };
        let id = field("id");
        if id.is_empty() {
            return Err("条目缺少 id".to_string());
        }
        let version = field("version");
        if version.is_empty() {
            return Err(format!("{id}：缺少 version"));
        }
        let sha256 = field("sha256").to_ascii_lowercase();
        if sha256.len() != 64 || !sha256.chars().all(|c| c.is_ascii_hexdigit()) {
            return Err(format!("{id}：sha256 缺失或格式错误"));
        }
        let url = field("url");
        if url.is_empty() {
            return Err(format!("{id}：缺少 url"));
        }
        Ok(Self {
            id,
            name: field("name"),
            category: field("category"),
            version,
            target: {
                let t = field("target");
                if t.is_empty() { "any".to_string() } else { t }
            },
            sha256,
            url,
            size_kb: item.get("sizeKB").and_then(|v| v.as_i64()).unwrap_or(0),
            notes: field("notes"),
        })
    }
}

/// 已安装的数据集。
#[derive(Clone, Debug)]
pub struct InstalledDb {
    /// 数据集 id
    pub id: String,
    /// 版本
    pub version: String,
    /// 解压目录
    pub dir: PathBuf,
    /// 数据文件（xdb/mmdb 等）
    pub files: Vec<PathBuf>,
    /// 包 SHA-256
    pub sha256: String,
    /// 安装时间（Unix 秒）
    pub installed_at: u64,
}

impl InstalledDb {
    /// 按扩展名查找第一个文件（`ext` 可带或不带 `.`，大小写不敏感）。
    pub fn file_with_ext(&self, ext: &str) -> Option<&Path> {
        let ext = ext.trim().trim_start_matches('.').to_ascii_lowercase();
        self.files
            .iter()
            .find(|path| {
                path.extension()
                    .and_then(|e| e.to_str())
                    .is_some_and(|e| e.eq_ignore_ascii_case(&ext))
            })
            .map(|path| path.as_path())
    }

    /// 按扩展名查找全部文件。
    pub fn files_with_ext(&self, ext: &str) -> Vec<&Path> {
        let ext = ext.trim().trim_start_matches('.').to_ascii_lowercase();
        self.files
            .iter()
            .filter(|path| {
                path.extension()
                    .and_then(|e| e.to_str())
                    .is_some_and(|e| e.eq_ignore_ascii_case(&ext))
            })
            .map(PathBuf::as_path)
            .collect()
    }

    /// 主数据文件（第一个 `.xdb` 或 `.mmdb`）。
    pub fn primary_file(&self) -> Option<&Path> {
        self.file_with_ext("xdb")
            .or_else(|| self.file_with_ext("mmdb"))
    }
}

/// IP 数据管理器。
#[derive(Debug)]
pub struct IpDbManager {
    cfg: IpDbManagerConfig,
    cache_dir: PathBuf,
    catalog_url: String,
    origin: String,
    /// 授权运行时（首次下载时惰性创建；单飞锁避免重复注册）
    auth: Mutex<Option<AuthRuntime>>,
}

impl IpDbManager {
    /// 创建管理器。
    pub fn new(cfg: IpDbManagerConfig) -> Result<Self> {
        let store = cfg.store_url.trim().trim_end_matches('/').to_string();
        let has_pubkey = !cfg.pubkey.trim().is_empty();
        if !store.is_empty() && !url_allowed(&store, has_pubkey) {
            return Err(Error::Argument(format!(
                "组件源地址不被允许（需 https；配置公钥后可放行内网 http）：{store}"
            )));
        }
        if has_pubkey {
            dhrust::plugin::parse_pubkey(cfg.pubkey.trim())
                .map_err(|e| Error::Argument(format!("公钥无效（{e}）")))?;
        }
        let (catalog_url, origin) = if store.is_empty() {
            (String::new(), String::new())
        } else if store.ends_with(".json") {
            (store.clone(), origin_of(&store))
        } else {
            (format!("{store}{CATALOG_PATH}"), origin_of(&store))
        };
        let cache_dir = cfg
            .cache_dir
            .clone()
            .unwrap_or_else(|| std::env::temp_dir().join(DEFAULT_CACHE_DIR));
        Ok(Self {
            cfg,
            cache_dir,
            catalog_url,
            origin,
            auth: Mutex::new(None),
        })
    }

    /// 缓存目录。
    pub fn cache_dir(&self) -> &Path {
        &self.cache_dir
    }

    /// 是否联网模式。
    pub fn is_online(&self) -> bool {
        !self.catalog_url.is_empty()
    }

    /// 拉取组件源目录（联网模式）。
    pub fn catalog(&self) -> Result<Vec<ComponentEntry>> {
        if !self.is_online() {
            return Err(Error::Store("未配置组件源（仅本地缓存模式）".to_string()));
        }
        let bytes = self.http_get(&self.catalog_url, MAX_CATALOG_BYTES, "组件目录")?;
        if !self.cfg.pubkey.trim().is_empty() {
            let sig_url = format!("{}.sig", self.catalog_url);
            let sig = self.http_get(&sig_url, MAX_SIGNATURE_BYTES, "目录签名")?;
            let sig_text = String::from_utf8_lossy(&sig);
            dhrust::plugin::verify_base64(self.cfg.pubkey.trim(), &bytes, sig_text.trim())
                .map_err(|e| Error::Verify(format!("组件目录签名校验失败：{e}")))?;
        }
        let text = String::from_utf8(bytes)
            .map_err(|e| Error::Store(format!("组件目录不是 UTF-8：{e}")))?;
        let root: Json = serde_json::from_str(&text)
            .map_err(|e| Error::Store(format!("组件目录 JSON 解析失败：{e}")))?;
        let items = root
            .get("components")
            .and_then(|v| v.as_array())
            .ok_or_else(|| Error::Store("组件目录缺少 components 数组".to_string()))?;
        let mut out = Vec::with_capacity(items.len());
        for item in items {
            match ComponentEntry::from_json(item) {
                Ok(entry) => out.push(entry),
                Err(err) => {
                    // 单条无效不影响其它条目（与驱动/插件源一致）
                    let _ = err;
                }
            }
        }
        Ok(out)
    }

    /// 确保数据集可用：本地有任意版本则直接返回（离线优先），否则联网下载。
    pub fn ensure(&self, dataset_id: &str) -> Result<InstalledDb> {
        if let Some(local) = self.find_local_latest(dataset_id)? {
            return Ok(local);
        }
        self.ensure_updated(dataset_id)
    }

    /// 检查更新：联网拉目录，下载/安装最新版本；本地已是最新则直接返回。
    pub fn ensure_updated(&self, dataset_id: &str) -> Result<InstalledDb> {
        if !self.is_online() {
            return self.find_local_latest(dataset_id)?.ok_or_else(|| {
                Error::DatasetNotFound(format!(
                    "{dataset_id}（未配置组件源，且本地缓存没有可用版本）"
                ))
            });
        }
        let entries = self.catalog()?;
        let entry = self.select_entry(&entries, dataset_id)?;
        if let Ok(local) = self.load_installed(&entry.id, &entry.version)
            && local.sha256.eq_ignore_ascii_case(&entry.sha256)
        {
            return Ok(local);
        }
        self.download_and_install(entry)
    }

    /// 列出本地已安装的数据集（每个版本一条）。
    pub fn list_local(&self) -> Result<Vec<InstalledDb>> {
        let mut out = Vec::new();
        let root = &self.cache_dir;
        if !root.is_dir() {
            return Ok(out);
        }
        for entry in fs::read_dir(root)? {
            let entry = entry?;
            if !entry.file_type()?.is_dir() {
                continue;
            }
            let id = entry.file_name().to_string_lossy().to_string();
            let id_dir = entry.path();
            for version_entry in fs::read_dir(&id_dir)? {
                let version_entry = version_entry?;
                if !version_entry.file_type()?.is_dir() {
                    continue;
                }
                let version = version_entry.file_name().to_string_lossy().to_string();
                if let Ok(db) = self.load_installed(&id, &version) {
                    out.push(db);
                }
            }
        }
        out.sort_by(|a, b| {
            a.id.cmp(&b.id)
                .then_with(|| dhrust::version::compare_numeric(&a.version, &b.version))
        });
        Ok(out)
    }

    /// 查找某数据集的本地最新版本。
    pub fn find_local_latest(&self, dataset_id: &str) -> Result<Option<InstalledDb>> {
        let dir = self.cache_dir.join(dataset_id);
        if !dir.is_dir() {
            return Ok(None);
        }
        let mut versions = Vec::new();
        for entry in fs::read_dir(&dir)? {
            let entry = entry?;
            if !entry.file_type()?.is_dir() {
                continue;
            }
            let version = entry.file_name().to_string_lossy().to_string();
            if self.load_installed(dataset_id, &version).is_ok() {
                versions.push(version);
            }
        }
        versions.sort_by(|a, b| dhrust::version::compare_numeric(a, b));
        if let Some(version) = versions.pop() {
            return self.load_installed(dataset_id, &version).map(Some);
        }
        Ok(None)
    }

    /// 从本地 zip 安装（离线部署/测试）。
    pub fn install_from_zip(
        &self,
        dataset_id: &str,
        version: &str,
        zip_path: impl AsRef<Path>,
    ) -> Result<InstalledDb> {
        validate_id(dataset_id)?;
        validate_version(version)?;
        let zip_path = zip_path.as_ref();
        if !zip_path.is_file() {
            return Err(Error::MissingFile(zip_path.to_path_buf()));
        }
        let data = fs::read(zip_path)?;
        let sha256 = dhrust::sign::sha256_hex(&data);
        self.install_bytes(dataset_id, version, &sha256, &data)
    }

    /// 启动时构建定位器：先查本地 `data/`，缺失再按需下载。
    ///
    /// 本地文件命名约定（不区分大小写）：
    ///
    /// - IPv4：`ip2region_v4.xdb`（兼容 `ip2region.xdb`）
    /// - IPv6：`ip2region_v6.xdb`
    /// - 城市/国家：`*.mmdb` 且名称含 `city` / `country`
    /// - ASN/ISP：`*.mmdb` 且名称含 `asn` / `isp`
    ///
    /// 缺失时按数据集 id 调用 [`IpDbManager::ensure`]（本地缓存优先，联网才下载）。
    pub fn build_locator(&self, options: LocatorLoadOptions) -> Result<IpLocator> {
        #[cfg(not(feature = "mmdb"))]
        if options.load_world || options.load_asn {
            return Err(Error::Argument(
                "加载国外数据需要启用 mmdb 特性".to_string(),
            ));
        }

        let data_dir = options.data_dir.as_path();

        let v4 = if options.load_v4 {
            let local = local_xdb(data_dir, &["ip2region_v4.xdb", "ip2region.xdb"]);
            let path = optional_dataset(
                self.resolve_local_or_download(
                    DATASET_IP2REGION_V4,
                    local,
                    options.download_missing,
                ),
                options.allow_partial,
                DATASET_IP2REGION_V4,
            )?;
            match path {
                Some(path) => optional_dataset(
                    XdbReader::open(path).map(Some),
                    options.allow_partial,
                    DATASET_IP2REGION_V4,
                )?,
                None => None,
            }
        } else {
            None
        };

        let v6 = if options.load_v6 {
            let local = local_xdb(data_dir, &["ip2region_v6.xdb"]);
            let path = optional_dataset(
                self.resolve_local_or_download(DATASET_IP2REGION_V6, local, options.download_v6),
                options.allow_partial,
                DATASET_IP2REGION_V6,
            )?;
            match path {
                Some(path) => optional_dataset(
                    XdbReader::open(path).map(Some),
                    options.allow_partial,
                    DATASET_IP2REGION_V6,
                )?,
                None => None,
            }
        } else {
            None
        };

        #[cfg(feature = "mmdb")]
        let world = if options.load_world {
            let local = local_mmdb(data_dir, |name| {
                name.contains("city") || name.contains("country")
            });
            let path = optional_dataset(
                self.resolve_local_or_download(
                    DATASET_GEOLITE2_CITY,
                    local,
                    options.download_missing,
                ),
                options.allow_partial,
                DATASET_GEOLITE2_CITY,
            )?;
            match path {
                Some(path) => optional_dataset(
                    MmdbReader::open_auto(path).map(Some),
                    options.allow_partial,
                    DATASET_GEOLITE2_CITY,
                )?,
                None => None,
            }
        } else {
            None
        };

        #[cfg(feature = "mmdb")]
        let asn = if options.load_asn {
            let local = local_mmdb(data_dir, |name| {
                name.contains("asn") || name.contains("isp")
            });
            let path = optional_dataset(
                self.resolve_local_or_download(
                    DATASET_GEOLITE2_ASN,
                    local,
                    options.download_missing,
                ),
                options.allow_partial,
                DATASET_GEOLITE2_ASN,
            )?;
            match path {
                Some(path) => optional_dataset(
                    MmdbReader::open_auto(path).map(Some),
                    options.allow_partial,
                    DATASET_GEOLITE2_ASN,
                )?,
                None => None,
            }
        } else {
            None
        };

        let mut builder = IpLocator::builder();
        if let Some(reader) = v4 {
            if reader.ip_version() == crate::xdb::XdbIpVersion::V4 {
                builder = builder.xdb_v4_reader(reader)?;
            } else if !options.allow_partial {
                return Err(Error::Argument(format!(
                    "{} 不是 IPv4 xdb",
                    DATASET_IP2REGION_V4
                )));
            }
        }
        if let Some(reader) = v6 {
            if reader.ip_version() == crate::xdb::XdbIpVersion::V6 {
                builder = builder.xdb_v6_reader(reader)?;
            } else if !options.allow_partial {
                return Err(Error::Argument(format!(
                    "{} 不是 IPv6 xdb",
                    DATASET_IP2REGION_V6
                )));
            }
        }
        #[cfg(feature = "mmdb")]
        {
            if let Some(reader) = world {
                builder = builder.world_reader(reader)?;
            }
            if let Some(reader) = asn {
                builder = builder.world_asn_reader(reader)?;
            }
        }
        Ok(builder.build())
    }

    fn resolve_local_or_download(
        &self,
        dataset_id: &str,
        local: Option<PathBuf>,
        download: bool,
    ) -> Result<Option<PathBuf>> {
        if let Some(path) = local {
            return Ok(Some(path));
        }
        if !download {
            return Ok(None);
        }
        let db = self.ensure(dataset_id)?;
        Ok(db.primary_file().map(Path::to_path_buf))
    }

    fn select_entry<'a>(
        &self,
        entries: &'a [ComponentEntry],
        dataset_id: &str,
    ) -> Result<&'a ComponentEntry> {
        let current = self
            .cfg
            .target
            .clone()
            .unwrap_or_else(current_target_or_unknown);
        let mut candidates: Vec<&ComponentEntry> = entries
            .iter()
            .filter(|e| e.id.eq_ignore_ascii_case(dataset_id))
            .collect();
        if candidates.is_empty() {
            return Err(Error::DatasetNotFound(dataset_id.to_string()));
        }
        candidates.retain(|e| e.target == "any" || e.target.eq_ignore_ascii_case(&current));
        if candidates.is_empty() {
            return Err(Error::DatasetNotFound(format!(
                "{dataset_id}（没有适用于 {current}/any 的包）"
            )));
        }
        // any 优先，其次当前平台；同优先级取最高版本
        candidates.sort_by(|a, b| {
            let pa = if a.target == "any" { 0 } else { 1 };
            let pb = if b.target == "any" { 0 } else { 1 };
            pa.cmp(&pb)
                .then_with(|| dhrust::version::compare_numeric(&b.version, &a.version))
        });
        Ok(candidates[0])
    }

    /// 申请授权下载 URL：返回 `None` = 使用目录中的公开 URL（未启用授权或旧平台不支持）。
    fn authorized_url(&self, entry: &ComponentEntry) -> Result<Option<String>> {
        if !self.cfg.auth.enabled || self.origin.is_empty() {
            return Ok(None);
        }
        let mut guard = self
            .auth
            .lock()
            .map_err(|_| Error::Auth("授权状态锁已损坏".to_string()))?;
        if guard.is_none() {
            let client = AuthClient::load_or_create(&self.cfg.auth, &self.cache_dir)?;
            *guard = Some(AuthRuntime {
                client,
                unsupported: false,
            });
        }
        let runtime = guard.as_mut().expect("auth runtime initialized");
        if runtime.unsupported {
            return Ok(None);
        }
        match runtime
            .client
            .ticket(&self.origin, &entry.id, &entry.version)
        {
            Ok(url) => Ok(Some(url)),
            Err(Error::Unsupported(_)) => {
                // 旧平台没有 `/api/ipdb/*`：退回公开下载（新平台仍由票据保护）
                runtime.unsupported = true;
                Ok(None)
            }
            Err(err) => Err(err),
        }
    }

    fn download_and_install(&self, entry: &ComponentEntry) -> Result<InstalledDb> {
        let url = match self.authorized_url(entry)? {
            Some(url) => url,
            None => self.absolute_url(&entry.url),
        };
        fs::create_dir_all(&self.cache_dir)?;
        let package_path = self.cache_dir.join(format!(
            ".download-{}-{}-{}.zip",
            sanitize(&entry.id),
            sanitize(&entry.version),
            std::process::id()
        ));
        let result = self.download_package(&url, &package_path);
        let result = match result {
            Ok(()) => (|| {
                let data = fs::read(&package_path)?;
                let actual = dhrust::sign::sha256_hex(&data);
                if !actual.eq_ignore_ascii_case(&entry.sha256) {
                    return Err(Error::Verify(format!(
                        "数据包 SHA-256 不匹配：期望 {}，实际 {actual}",
                        entry.sha256
                    )));
                }
                self.install_bytes(&entry.id, &entry.version, &entry.sha256, &data)
            })(),
            Err(err) => Err(err),
        };
        let _ = fs::remove_file(&package_path);
        result
    }

    fn install_bytes(
        &self,
        dataset_id: &str,
        version: &str,
        sha256: &str,
        zip_data: &[u8],
    ) -> Result<InstalledDb> {
        validate_id(dataset_id)?;
        validate_version(version)?;
        if zip_data.len() as u64 > self.cfg.package_limit {
            return Err(Error::Verify(format!(
                "数据包超过上限（{} MiB）",
                self.cfg.package_limit / 1024 / 1024
            )));
        }
        if !zip_data.starts_with(b"PK") {
            return Err(Error::Verify("数据包不是 zip 文件".to_string()));
        }

        let dataset_dir = self.cache_dir.join(dataset_id);
        fs::create_dir_all(&dataset_dir)?;
        let temp_dir = dataset_dir.join(format!(
            ".install-{}-{}-{}",
            sanitize(version),
            std::process::id(),
            now_secs()
        ));
        let _ = fs::remove_dir_all(&temp_dir);
        fs::create_dir_all(&temp_dir)?;

        let temp_zip = temp_dir.join("package.zip");
        fs::write(&temp_zip, zip_data)?;
        let extract_dir = temp_dir.join("data");
        fs::create_dir_all(&extract_dir)?;
        dhrust::zip::extract_zip(&temp_zip, &extract_dir)
            .map_err(|e| Error::Verify(format!("解压数据包失败：{e}")))?;
        let _ = fs::remove_file(&temp_zip);

        let files = collect_data_files(&extract_dir)?;
        if files.is_empty() {
            let _ = fs::remove_dir_all(&temp_dir);
            return Err(Error::Verify("数据包中没有 .xdb 或 .mmdb 文件".to_string()));
        }
        let relative: Vec<String> = files
            .iter()
            .map(|p| {
                p.strip_prefix(&extract_dir)
                    .unwrap_or(p)
                    .to_string_lossy()
                    .to_string()
            })
            .collect();
        let installed_at = now_secs();
        write_marker(
            &extract_dir,
            dataset_id,
            version,
            sha256,
            installed_at,
            &relative,
        )?;

        let final_dir = dataset_dir.join(version);
        if final_dir.exists() {
            fs::remove_dir_all(&final_dir)?;
        }
        // 把 data/ 下的内容提升到版本目录根，保持路径与安装前一致
        let staged = dataset_dir.join(format!(
            ".ready-{}-{}-{}",
            sanitize(version),
            std::process::id(),
            now_secs()
        ));
        let _ = fs::remove_dir_all(&staged);
        fs::rename(&extract_dir, &staged)?;
        let _ = fs::remove_dir_all(&temp_dir);
        fs::rename(&staged, &final_dir)?;

        self.clean_old_versions(dataset_id, version)?;
        self.load_installed(dataset_id, version)
    }

    fn clean_old_versions(&self, dataset_id: &str, keep_version: &str) -> Result<()> {
        let keep = self.cfg.keep_versions.max(1);
        let dir = self.cache_dir.join(dataset_id);
        let mut versions = Vec::new();
        for entry in fs::read_dir(&dir)? {
            let entry = entry?;
            if !entry.file_type()?.is_dir() {
                continue;
            }
            let name = entry.file_name().to_string_lossy().to_string();
            if name.starts_with('.') {
                continue;
            }
            versions.push(name);
        }
        versions.sort_by(|a, b| dhrust::version::compare_numeric(a, b));
        while versions.len() > keep {
            let Some(oldest) = versions.first().cloned() else {
                break;
            };
            if oldest == keep_version {
                // 理论不会发生（最新版本应在末尾），防御性跳过
                versions.remove(0);
                continue;
            }
            let _ = fs::remove_dir_all(dir.join(&oldest));
            versions.remove(0);
        }
        Ok(())
    }

    fn load_installed(&self, dataset_id: &str, version: &str) -> Result<InstalledDb> {
        let dir = self.cache_dir.join(dataset_id).join(version);
        let marker = read_marker(&dir)?;
        if !marker.id.eq_ignore_ascii_case(dataset_id) {
            return Err(Error::Verify(format!(
                "缓存标记 id 不匹配：{} != {dataset_id}",
                marker.id
            )));
        }
        let mut files = Vec::new();
        for rel in &marker.files {
            let path = dir.join(rel);
            if path.is_file() {
                files.push(path);
            }
        }
        if files.is_empty() {
            return Err(Error::Verify(format!(
                "缓存目录缺少数据文件：{}",
                dir.display()
            )));
        }
        Ok(InstalledDb {
            id: marker.id,
            version: marker.version,
            dir,
            files,
            sha256: marker.sha256,
            installed_at: marker.installed_at,
        })
    }

    fn http_get(&self, url: &str, max: usize, what: &str) -> Result<Vec<u8>> {
        let options = self.http_options();
        let headers = self.headers();
        let refs: Vec<(&str, &str)> = headers
            .iter()
            .map(|(k, v)| (k.as_str(), v.as_str()))
            .collect();
        let resp = http_client::blocking_request_offthread_with(
            "GET",
            url,
            &refs,
            None,
            Vec::new(),
            options,
        )
        .map_err(|e| Error::Store(format!("{what}请求失败：{e}")))?;
        if !resp.is_success() {
            return Err(Error::Store(format!(
                "{what}请求失败：HTTP {} {}",
                resp.status,
                resp.body_text().chars().take(200).collect::<String>()
            )));
        }
        if resp.body.len() > max {
            return Err(Error::Store(format!("{what}超过大小上限（{} 字节）", max)));
        }
        Ok(resp.body)
    }

    fn download_package(&self, url: &str, dest: &Path) -> Result<()> {
        let options = self.http_options();
        let headers = self.headers();
        let dest = dest.to_path_buf();
        let limit = self.cfg.package_limit;
        let url = url.to_string();
        let (tx, rx) = mpsc::channel();
        std::thread::spawn(move || {
            let refs: Vec<(&str, &str)> = headers
                .iter()
                .map(|(k, v)| (k.as_str(), v.as_str()))
                .collect();
            let result = http_client::blocking_download_to_file(
                &url,
                &refs,
                &options,
                &dest,
                |downloaded, total| {
                    if total.is_some_and(|t| t > limit) {
                        return false;
                    }
                    downloaded <= limit
                },
            );
            let _ = tx.send(result);
        });
        let result = rx
            .recv()
            .map_err(|_| Error::Store("下载线程异常退出".to_string()))?;
        let result = result.map_err(|e| Error::Store(format!("数据包下载失败：{e}")))?;
        if !(200..300).contains(&result.status) {
            let body = String::from_utf8_lossy(&result.error_body);
            return Err(Error::Store(format!(
                "数据包下载失败：HTTP {} {}",
                result.status,
                body.chars().take(200).collect::<String>()
            )));
        }
        if result.len > limit {
            return Err(Error::Verify(format!(
                "数据包超过上限（{} MiB）",
                limit / 1024 / 1024
            )));
        }
        Ok(())
    }

    fn http_options(&self) -> HttpClientOptions {
        HttpClientOptions {
            timeout: self.cfg.timeout.unwrap_or(Duration::from_secs(60)),
            insecure_tls: false,
            ca_pem: self.cfg.ca_pem.clone(),
        }
    }

    fn headers(&self) -> Vec<(String, String)> {
        let mut headers = Vec::new();
        if let Some(token) = self.cfg.access_token.as_deref().map(str::trim)
            && !token.is_empty()
        {
            headers.push(("Authorization".to_string(), format!("Bearer {token}")));
        }
        headers
    }

    fn absolute_url(&self, url: &str) -> String {
        let url = url.trim();
        if url.starts_with("http://") || url.starts_with("https://") {
            return url.to_string();
        }
        if url.starts_with('/') {
            format!("{}{url}", self.origin.trim_end_matches('/'))
        } else {
            format!(
                "{}/{}",
                self.origin.trim_end_matches('/'),
                url.trim_start_matches('/')
            )
        }
    }
}

#[derive(Debug)]
struct Marker {
    id: String,
    version: String,
    sha256: String,
    installed_at: u64,
    files: Vec<String>,
}

fn write_marker(
    dir: &Path,
    id: &str,
    version: &str,
    sha256: &str,
    installed_at: u64,
    files: &[String],
) -> Result<()> {
    let value = json!({
        "schema": 1,
        "id": id,
        "version": version,
        "sha256": sha256,
        "installedAt": installed_at,
        "files": files,
    });
    let text = serde_json::to_string_pretty(&value)
        .map_err(|e| Error::Store(format!("序列化缓存标记失败：{e}")))?;
    fs::write(dir.join(MARKER_FILE), text)?;
    Ok(())
}

fn read_marker(dir: &Path) -> Result<Marker> {
    let path = dir.join(MARKER_FILE);
    let text = fs::read_to_string(&path)
        .map_err(|e| Error::Verify(format!("读取缓存标记失败 {}：{e}", path.display())))?;
    let value: Json = serde_json::from_str(&text)
        .map_err(|e| Error::Verify(format!("缓存标记 JSON 无效：{e}")))?;
    let field = |key: &str| {
        value
            .get(key)
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string()
    };
    let id = field("id");
    let version = field("version");
    let sha256 = field("sha256");
    if id.is_empty() || version.is_empty() {
        return Err(Error::Verify(format!(
            "缓存标记缺少 id/version：{}",
            path.display()
        )));
    }
    let files = value
        .get("files")
        .and_then(|v| v.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|v| v.as_str().map(str::to_string))
                .collect()
        })
        .unwrap_or_default();
    Ok(Marker {
        id,
        version,
        sha256,
        installed_at: value
            .get("installedAt")
            .and_then(|v| v.as_u64())
            .unwrap_or(0),
        files,
    })
}

impl IpLocator {
    /// 启动时一行加载：本地 `data/` 优先，缺失按需从组件源下载。
    ///
    /// ```no_run
    /// use pek_ip::{IpDbManagerConfig, IpLocator, LocatorLoadOptions};
    ///
    /// # fn main() -> Result<(), pek_ip::Error> {
    /// let locator = IpLocator::load_or_download(
    ///     IpDbManagerConfig {
    ///         store_url: "https://p.sc8.fun".into(),
    ///         pubkey: "<平台公钥 hex>".into(),
    ///         ..Default::default()
    ///     },
    ///     LocatorLoadOptions::default(),
    /// )?;
    /// # Ok(()) }
    /// ```
    pub fn load_or_download(store: IpDbManagerConfig, options: LocatorLoadOptions) -> Result<Self> {
        IpDbManager::new(store)?.build_locator(options)
    }
}

fn optional_dataset<T>(
    result: Result<Option<T>>,
    allow_partial: bool,
    dataset_id: &str,
) -> Result<Option<T>> {
    match result {
        Ok(Some(value)) => Ok(Some(value)),
        Ok(None) if allow_partial => Ok(None),
        Ok(None) => Err(Error::DatasetNotFound(dataset_id.to_string())),
        // 启动时允许“没数据/网络失败”，但安全/格式错误必须暴露
        Err(err) if allow_partial && matches!(err, Error::DatasetNotFound(_) | Error::Store(_)) => {
            Ok(None)
        }
        Err(err) => Err(err),
    }
}

fn local_xdb(dir: &Path, names: &[&str]) -> Option<PathBuf> {
    names
        .iter()
        .map(|name| dir.join(name))
        .find(|path| path.is_file())
}

fn local_mmdb(dir: &Path, predicate: impl Fn(&str) -> bool) -> Option<PathBuf> {
    let mut candidates: Vec<(u8, String, PathBuf)> = fs::read_dir(dir)
        .ok()?
        .flatten()
        .filter_map(|entry| {
            let path = entry.path();
            if !path.is_file() {
                return None;
            }
            let name = path.file_name()?.to_str()?.to_string();
            let lower = name.to_ascii_lowercase();
            if !lower.ends_with(".mmdb") || !predicate(&lower) {
                return None;
            }
            let priority = if lower.contains("city") {
                0
            } else if lower.contains("country") {
                1
            } else if lower.contains("asn") {
                0
            } else if lower.contains("isp") {
                1
            } else {
                2
            };
            Some((priority, lower, path))
        })
        .collect();
    candidates.sort_by(|a, b| a.0.cmp(&b.0).then_with(|| a.1.cmp(&b.1)));
    candidates.into_iter().next().map(|(_, _, path)| path)
}

fn collect_data_files(root: &Path) -> Result<Vec<PathBuf>> {
    let mut out = Vec::new();
    collect_files_recursive(root, &mut out)?;
    out.retain(|p| {
        p.extension()
            .and_then(|e| e.to_str())
            .is_some_and(|e| e.eq_ignore_ascii_case("xdb") || e.eq_ignore_ascii_case("mmdb"))
    });
    out.sort();
    Ok(out)
}

fn collect_files_recursive(dir: &Path, out: &mut Vec<PathBuf>) -> Result<()> {
    for entry in fs::read_dir(dir)? {
        let entry = entry?;
        let path = entry.path();
        if path.is_dir() {
            collect_files_recursive(&path, out)?;
        } else if path.is_file() {
            out.push(path);
        }
    }
    Ok(())
}

fn validate_id(id: &str) -> Result<()> {
    dhrust::plugin::validate_id(id).map_err(Error::Argument)
}

fn validate_version(version: &str) -> Result<()> {
    dhrust::plugin::validate_version(version).map_err(Error::Argument)
}

fn sanitize(text: &str) -> String {
    text.chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | '_') {
                c
            } else {
                '_'
            }
        })
        .collect()
}

fn now_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

fn url_allowed(url: &str, has_pubkey: bool) -> bool {
    let u = url.trim();
    if u.starts_with("https://") {
        return true;
    }
    if !u.starts_with("http://") {
        return false;
    }
    if has_pubkey {
        return true;
    }
    let host = &u["http://".len()..];
    host.starts_with("127.0.0.1") || host.starts_with("localhost") || host.starts_with("[::1]")
}

fn origin_of(url: &str) -> String {
    let Some((scheme, rest)) = url.split_once("://") else {
        return String::new();
    };
    let host = rest.split('/').next().unwrap_or(rest);
    format!("{scheme}://{host}")
}

/// 当前平台标识（数据包通常用 `any`，保留以兼容平台特定包）。
pub fn current_target() -> Option<&'static str> {
    Some(match (std::env::consts::OS, std::env::consts::ARCH) {
        ("windows", "x86_64") => "win-x64",
        ("windows", "aarch64") => "win-arm64",
        ("linux", "x86_64") => "linux-x64",
        ("linux", "aarch64") => "linux-arm64",
        ("linux", "riscv64") => "linux-riscv64",
        ("linux", "loongarch64") => "linux-loongarch64",
        ("macos", "x86_64") => "macos-x64",
        ("macos", "aarch64") => "macos-arm64",
        _ => return None,
    })
}

fn current_target_or_unknown() -> String {
    current_target().unwrap_or("any").to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{Read, Write};
    use std::net::TcpListener;

    fn temp_dir(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "pek-ip-test-{tag}-{}-{}",
            std::process::id(),
            now_secs()
        ));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn entry_validation() {
        let item = json!({
            "id": "ip2region-v4",
            "name": "ip2region IPv4",
            "category": "ip-database",
            "version": "2026.10",
            "target": "any",
            "sha256": "a".repeat(64),
            "url": "https://example.com/a.zip",
        });
        let entry = ComponentEntry::from_json(&item).unwrap();
        assert_eq!(entry.id, "ip2region-v4");
        assert_eq!(entry.target, "any");
        assert!(ComponentEntry::from_json(&json!({"id": "x"})).is_err());
    }

    #[test]
    fn url_policy() {
        assert!(url_allowed("https://p.sc8.fun", false));
        assert!(url_allowed("http://127.0.0.1:5502", false));
        assert!(!url_allowed("http://192.168.1.2:5502", false));
        assert!(url_allowed("http://192.168.1.2:5502", true));
        assert!(!url_allowed("ftp://x", true));
    }

    #[test]
    fn origin_and_absolute_url() {
        assert_eq!(origin_of("https://p.sc8.fun/a/b"), "https://p.sc8.fun");
        assert_eq!(origin_of("http://127.0.0.1:5502"), "http://127.0.0.1:5502");
        let mgr = IpDbManager::new(IpDbManagerConfig {
            store_url: "http://127.0.0.1:5502".into(),
            cache_dir: Some(temp_dir("abs")),
            ..Default::default()
        })
        .unwrap();
        assert_eq!(
            mgr.absolute_url("/components/a.zip"),
            "http://127.0.0.1:5502/components/a.zip"
        );
        assert_eq!(mgr.absolute_url("a.zip"), "http://127.0.0.1:5502/a.zip");
        assert_eq!(mgr.absolute_url("https://x/a.zip"), "https://x/a.zip");
    }

    #[test]
    fn install_from_zip_and_cache_only() {
        let base = temp_dir("install");
        let zip_path = base.join("pkg.zip");
        let mut zip = dhrust::zip::ZipWriter::new();
        zip.add_file("ip2region_v4.xdb", b"fake-xdb");
        fs::write(&zip_path, zip.finish()).unwrap();

        let cache = base.join("cache");
        let mgr = IpDbManager::new(IpDbManagerConfig {
            cache_dir: Some(cache.clone()),
            ..Default::default()
        })
        .unwrap();
        let db = mgr
            .install_from_zip("ip2region-v4", "2026.10.1", &zip_path)
            .unwrap();
        assert_eq!(db.version, "2026.10.1");
        assert_eq!(db.files.len(), 1);
        assert!(db.file_with_ext("xdb").is_some());
        assert!(db.primary_file().is_some());

        // 仅本地缓存模式（store_url 为空）
        let got = mgr.ensure("ip2region-v4").unwrap();
        assert_eq!(got.version, "2026.10.1");
        let latest = mgr.find_local_latest("ip2region-v4").unwrap().unwrap();
        assert_eq!(latest.version, "2026.10.1");
        assert_eq!(mgr.list_local().unwrap().len(), 1);

        // 新版本安装后只保留 1 个版本
        let zip2 = base.join("pkg2.zip");
        let mut zip = dhrust::zip::ZipWriter::new();
        zip.add_file("ip2region_v4.xdb", b"fake-xdb-2");
        fs::write(&zip2, zip.finish()).unwrap();
        let db2 = mgr
            .install_from_zip("ip2region-v4", "2026.11.1", &zip2)
            .unwrap();
        assert_eq!(db2.version, "2026.11.1");
        assert_eq!(mgr.list_local().unwrap().len(), 1);

        let _ = fs::remove_dir_all(&base);
    }

    #[test]
    fn online_download_verifies_signature_and_sha256() {
        let base = temp_dir("online");
        let key_hex = dhrust::plugin::generate_signing_key();
        let key = dhrust::plugin::signing_key_from_hex(&key_hex).unwrap();
        let pubkey = dhrust::plugin::pubkey_hex(&key);

        let mut zip = dhrust::zip::ZipWriter::new();
        zip.add_file("test.xdb", b"fake-xdb-for-online");
        let zip = zip.finish();
        let sha = dhrust::sign::sha256_hex(&zip);

        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        let origin = format!("http://{addr}");
        let catalog = json!({
            "name": "Pek 组件源",
            "version": 1,
            "components": [{
                "id": "test-db",
                "name": "测试数据",
                "category": "ip-database",
                "version": "1.0.0",
                "target": "any",
                "sha256": sha,
                "url": format!("{origin}/components/test.zip"),
                "sizeKB": 1,
            }],
        });
        let catalog_text = catalog.to_string();
        let signature = dhrust::plugin::sign_base64(&key, catalog_text.as_bytes());
        let responses = [
            (
                "/components/catalog.json".to_string(),
                catalog_text.into_bytes(),
            ),
            (
                "/components/catalog.json.sig".to_string(),
                signature.into_bytes(),
            ),
            ("/components/test.zip".to_string(), zip),
        ];
        let handle = std::thread::spawn(move || {
            for _ in 0..3 {
                let (mut stream, _) = listener.accept().unwrap();
                let mut buf = [0u8; 4096];
                let n = stream.read(&mut buf).unwrap_or(0);
                let request = String::from_utf8_lossy(&buf[..n]);
                let path = request
                    .lines()
                    .next()
                    .and_then(|line| line.split_whitespace().nth(1))
                    .unwrap_or("/");
                let body = responses
                    .iter()
                    .find(|(p, _)| p == path)
                    .map(|(_, b)| b.as_slice())
                    .unwrap_or(&[]);
                let header = format!(
                    "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                    body.len()
                );
                let _ = stream.write_all(header.as_bytes());
                let _ = stream.write_all(body);
            }
        });

        let cache = base.join("cache");
        let mgr = IpDbManager::new(IpDbManagerConfig {
            store_url: origin,
            pubkey,
            cache_dir: Some(cache),
            ..Default::default()
        })
        .unwrap();
        let db = mgr.ensure_updated("test-db").unwrap();
        assert_eq!(db.version, "1.0.0");
        assert!(db.file_with_ext("xdb").is_some());

        handle.join().unwrap();
        let _ = fs::remove_dir_all(&base);
    }

    #[test]
    fn build_locator_prefers_local_data() {
        let base = temp_dir("boot-local");
        let data = base.join("data");
        fs::create_dir_all(&data).unwrap();
        let xdb = crate::xdb::tests::build_xdb_v4(
            &[(0x0100_0000, 0x0100_ffff, "中国|福建省|福州市|中国电信|CN")],
            1,
        );
        fs::write(data.join("ip2region_v4.xdb"), xdb).unwrap();

        let mgr = IpDbManager::new(IpDbManagerConfig {
            cache_dir: Some(base.join("cache")),
            ..Default::default()
        })
        .unwrap();
        let locator = mgr
            .build_locator(LocatorLoadOptions {
                data_dir: data,
                load_v6: false,
                load_world: false,
                load_asn: false,
                download_missing: false,
                allow_partial: false,
                ..Default::default()
            })
            .unwrap();
        assert!(locator.has_china_db());
        let hit = locator.lookup_str("1.0.1.0").unwrap();
        assert_eq!(hit.city.as_deref(), Some("福州市"));
        let _ = fs::remove_dir_all(&base);
    }

    #[test]
    fn build_locator_missing_partial_or_strict() {
        let base = temp_dir("boot-missing");
        let data = base.join("data");
        fs::create_dir_all(&data).unwrap();
        let mgr = IpDbManager::new(IpDbManagerConfig {
            cache_dir: Some(base.join("cache")),
            ..Default::default()
        })
        .unwrap();
        let options = LocatorLoadOptions {
            data_dir: data,
            load_v6: false,
            load_world: false,
            load_asn: false,
            download_missing: false,
            ..Default::default()
        };
        let locator = mgr.build_locator(options.clone()).unwrap();
        assert!(!locator.has_china_db());

        let strict = LocatorLoadOptions {
            allow_partial: false,
            ..options
        };
        let err = mgr.build_locator(strict).unwrap_err();
        assert!(matches!(err, Error::DatasetNotFound(_)));
        let _ = fs::remove_dir_all(&base);
    }
}
