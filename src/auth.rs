//! 下载授权（零配置自动注册 + Ed25519 票据）。
//!
//! 业务系统不需要配置任何 clientId/密钥：首次运行时在 `state_dir` 自动生成
//! `install.id` 与 Ed25519 私钥，向平台 `/api/ipdb/enroll` 注册；平台按来源 CIDR
//! 自动批准或管理员手动批准后下发 License。下载数据包前，客户端用私钥签一张
//! 短期票据（`/api/ipdb/ticket`），平台验签、查配额后返回一次性下载 URL。
//!
//! 平台不支持授权接口（旧版本）时自动回退为公开下载；授权接口明确返回
//! 403/401 时不会回退（防止绕过平台策略）。

use std::fs;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};
use serde_json::{Value as Json, json};

use crate::{Error, Result};

/// 授权配置（业务系统零身份配置；仅需平台地址 + 公钥）。
#[derive(Clone, Debug, Default)]
pub struct IpAuthConfig {
    /// 是否启用自动注册/票据（默认 false；`Config/Ip.toml` 的 `AutoEnroll` 默认 true）。
    pub enabled: bool,
    /// 客户端状态目录（install.id / 私钥 / License）；缺省 = 缓存目录下 `.client`。
    pub state_dir: Option<PathBuf>,
    /// 显式指定 installId（一般留空 = 自动生成并持久化）。
    pub install_id: Option<String>,
    /// 客户端私钥文件（缺省 = `state_dir/client.key`）。
    pub client_key_file: Option<PathBuf>,
    /// License 文件（缺省 = `state_dir/license.json`）。
    pub license_file: Option<PathBuf>,
    /// 平台公钥（一般来自 `Config/Ip.toml` 的 `PubKey`；空 = 首次注册 TOFU 并持久化）。
    pub platform_pubkey: Option<String>,
    /// HTTP 超时（默认 30 秒）。
    pub timeout: Option<Duration>,
    /// 自定义 CA PEM（自签/内网平台）。
    pub ca_pem: Option<Vec<u8>>,
}

impl IpAuthConfig {
    /// 启用自动授权（使用默认状态目录与配置中的平台公钥）。
    pub fn enabled_with_pubkey(pubkey: impl Into<String>) -> Self {
        Self {
            enabled: true,
            platform_pubkey: Some(pubkey.into()),
            ..Default::default()
        }
    }
}

/// 平台下发的 License。
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct License {
    /// 许可证 id
    pub license_id: String,
    /// installId
    pub install_id: String,
    /// 客户端公钥
    pub client_pub_key: String,
    /// 允许的数据集
    pub datasets: Vec<String>,
    /// 授权模式
    pub auth_mode: String,
    /// 签发时间
    pub issued_at: i64,
    /// 到期时间
    pub not_after: i64,
    /// 每日配额
    pub daily_quota: i64,
}

impl License {
    /// 待签名字节（与平台端逐字节一致）。
    pub fn canonical(&self) -> String {
        format!(
            "pek-ip-license-v1\n{}\n{}\n{}\n{}\n{}\n{}\n{}\n{}",
            self.license_id,
            self.install_id,
            self.client_pub_key,
            self.datasets.join(","),
            self.auth_mode,
            self.issued_at,
            self.not_after,
            self.daily_quota
        )
    }

    /// 是否覆盖某数据集。
    pub fn allows_dataset(&self, dataset: &str) -> bool {
        self.datasets.is_empty() || self.datasets.iter().any(|d| d == dataset)
    }
}

/// 持久化的 License 文件。
#[derive(Clone, Debug, Serialize, Deserialize)]
struct LicenseFile {
    license: License,
    signature: String,
    pubkey: String,
}

/// 授权运行时（installId + 密钥 + License）。
pub struct AuthClient {
    install_id: String,
    key_hex: String,
    pubkey: String,
    state_dir: PathBuf,
    license_path: PathBuf,
    license: Option<License>,
    platform_pubkey: Option<String>,
    timeout: Duration,
    ca_pem: Option<Vec<u8>>,
}

impl std::fmt::Debug for AuthClient {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AuthClient")
            .field("install_id", &self.install_id)
            .field("pubkey", &self.pubkey)
            .field("state_dir", &self.state_dir)
            .field("has_license", &self.license.is_some())
            .finish_non_exhaustive()
    }
}

impl AuthClient {
    /// 加载或创建客户端身份（installId + 私钥）。
    pub fn load_or_create(cfg: &IpAuthConfig, cache_dir: &Path) -> Result<Self> {
        let state_dir = cfg
            .state_dir
            .clone()
            .unwrap_or_else(|| cache_dir.join(".client"));
        fs::create_dir_all(&state_dir)?;
        let install_id = match cfg.install_id.as_deref().map(str::trim) {
            Some(id) if !id.is_empty() => id.to_string(),
            _ => {
                let path = state_dir.join("install.id");
                match fs::read_to_string(&path) {
                    Ok(text) if !text.trim().is_empty() => text.trim().to_string(),
                    _ => {
                        let id = dhrust::random::hex(16);
                        write_private(&path, id.as_bytes())?;
                        id
                    }
                }
            }
        };
        if !valid_install_id(&install_id) {
            return Err(Error::Auth(format!(
                "installId 不合法（8~64 位 [A-Za-z0-9._-]）：{install_id}"
            )));
        }
        let key_path = cfg
            .client_key_file
            .clone()
            .unwrap_or_else(|| state_dir.join("client.key"));
        let key_hex = match fs::read_to_string(&key_path) {
            Ok(text) if !text.trim().is_empty() => text.trim().to_string(),
            _ => {
                let hex = dhrust::plugin::generate_signing_key();
                write_private(&key_path, hex.as_bytes())?;
                hex
            }
        };
        let signing_key = dhrust::plugin::signing_key_from_hex(&key_hex)
            .map_err(|e| Error::Auth(format!("客户端私钥无效：{e}")))?;
        let pubkey = dhrust::plugin::pubkey_hex(&signing_key);
        let license_path = cfg
            .license_file
            .clone()
            .unwrap_or_else(|| state_dir.join("license.json"));
        let license = read_license(&license_path).ok().flatten();
        let platform_pubkey = cfg
            .platform_pubkey
            .as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(str::to_string)
            .or_else(|| {
                fs::read_to_string(state_dir.join("pubkey.txt"))
                    .ok()
                    .map(|s| s.trim().to_string())
                    .filter(|s| !s.is_empty())
            });
        Ok(Self {
            install_id,
            key_hex,
            pubkey,
            state_dir,
            license_path,
            license,
            platform_pubkey,
            timeout: cfg.timeout.unwrap_or(Duration::from_secs(30)),
            ca_pem: cfg.ca_pem.clone(),
        })
    }

    /// installId。
    pub fn install_id(&self) -> &str {
        &self.install_id
    }

    /// 客户端公钥（hex）。
    pub fn pubkey(&self) -> &str {
        &self.pubkey
    }

    /// 当前 License（可能过期）。
    pub fn license(&self) -> Option<&License> {
        self.license.as_ref()
    }

    /// 平台公钥（配置或 TOFU 持久化）。
    pub fn platform_pubkey(&self) -> Option<&str> {
        self.platform_pubkey.as_deref()
    }

    /// 注册（幂等）；返回 whether approved。
    pub fn enroll(&mut self, origin: &str) -> Result<()> {
        let url = format!("{origin}/api/ipdb/enroll");
        let body = json!({
            "installId": self.install_id,
            "clientPubKey": self.pubkey,
            "hostname": hostname(),
            "appName": std::env::var("PEK_IP_APP_NAME").unwrap_or_default(),
            "machineId": machine_id(),
            "clientVersion": env!("CARGO_PKG_VERSION"),
        });
        let data = self.post_json(&url, body)?;
        self.apply_enroll_data(&data)?;
        if !self.license_valid() {
            let msg = data
                .get("message")
                .and_then(|v| v.as_str())
                .unwrap_or("等待平台管理员批准");
            return Err(Error::Auth(format!(
                "{}（installId={}）",
                msg, self.install_id
            )));
        }
        Ok(())
    }

    /// 确保 License 有效（过期/缺失时自动重新注册）。
    pub fn ensure_license(&mut self, origin: &str) -> Result<()> {
        if self.license_valid() {
            return Ok(());
        }
        self.enroll(origin)
    }

    /// 申请短期下载票据，返回平台签发的一次性 URL。
    pub fn ticket(&mut self, origin: &str, dataset: &str, version: &str) -> Result<String> {
        self.ensure_license(origin)?;
        let nonce = dhrust::random::hex(16);
        let ts = now_secs();
        let canonical = format!(
            "pek-ip-ticket-v1\n{}\n{dataset}\n{version}\n{nonce}\n{ts}",
            self.install_id
        );
        let key = dhrust::plugin::signing_key_from_hex(&self.key_hex)
            .map_err(|e| Error::Auth(format!("客户端私钥无效：{e}")))?;
        let signature = dhrust::plugin::sign_base64(&key, canonical.as_bytes());
        let url = format!("{origin}/api/ipdb/ticket");
        let body = json!({
            "installId": self.install_id,
            "dataset": dataset,
            "version": version,
            "nonce": nonce,
            "ts": ts,
            "signature": signature,
        });
        let data = self.post_json(&url, body)?;
        let ticket_url = data
            .get("url")
            .and_then(|v| v.as_str())
            .ok_or_else(|| Error::Auth("平台票据响应缺少 url".to_string()))?;
        validate_ticket_url(ticket_url, origin)?;
        Ok(ticket_url.to_string())
    }

    fn post_json(&self, url: &str, body: Json) -> Result<Json> {
        let bytes =
            serde_json::to_vec(&body).map_err(|e| Error::Auth(format!("请求序列化失败：{e}")))?;
        let options = dhrust::net::http_client::HttpClientOptions {
            timeout: self.timeout,
            insecure_tls: false,
            ca_pem: self.ca_pem.clone(),
        };
        let resp = dhrust::net::http_client::blocking_request_offthread_with(
            "POST",
            url,
            &[],
            Some("application/json"),
            bytes,
            options,
        )
        .map_err(|e| Error::Auth(format!("请求平台失败：{e}")))?;
        if resp.status == 404 || resp.status == 405 {
            return Err(Error::Unsupported(format!(
                "平台未启用 IP 下载授权（HTTP {}）",
                resp.status
            )));
        }
        let text = String::from_utf8_lossy(&resp.body);
        let root: Json = serde_json::from_str(&text)
            .map_err(|e| Error::Auth(format!("平台响应不是 JSON（HTTP {}）：{e}", resp.status)))?;
        let code = root.get("code").and_then(|v| v.as_i64()).unwrap_or(-1);
        let message = root
            .get("message")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();
        if resp.status >= 400 || code != 0 {
            let detail = if message.is_empty() {
                format!("HTTP {}", resp.status)
            } else {
                message
            };
            return Err(Error::Auth(detail));
        }
        Ok(root.get("data").cloned().unwrap_or(Json::Null))
    }

    fn apply_enroll_data(&mut self, data: &Json) -> Result<()> {
        let pubkey = data
            .get("pubkey")
            .and_then(|v| v.as_str())
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .ok_or_else(|| Error::Auth("平台响应缺少公钥".to_string()))?
            .to_ascii_lowercase();
        // 公钥固定：配置公钥或首次 TOFU 后，平台公钥变化一律拒绝（防中间人替换）
        if let Some(expected) = &self.platform_pubkey {
            if !expected.eq_ignore_ascii_case(&pubkey) {
                return Err(Error::Auth(
                    "平台公钥与已信任公钥不一致（可能被中间人替换）".to_string(),
                ));
            }
        } else {
            dhrust::plugin::parse_pubkey(&pubkey)
                .map_err(|e| Error::Auth(format!("平台公钥无效：{e}")))?;
            write_private(
                self.state_dir.join("pubkey.txt").as_path(),
                pubkey.as_bytes(),
            )?;
            self.platform_pubkey = Some(pubkey.clone());
        }
        let status = data.get("status").and_then(|v| v.as_str()).unwrap_or("");
        if status == "revoked" {
            return Err(Error::Auth(
                data.get("message")
                    .and_then(|v| v.as_str())
                    .unwrap_or("客户端已被平台吊销")
                    .to_string(),
            ));
        }
        let Some(license_value) = data.get("license").filter(|v| !v.is_null()) else {
            // pending：不覆盖本地旧 License（可能仍有效），但本次注册未获批准
            return Ok(());
        };
        let signature = data
            .get("signature")
            .and_then(|v| v.as_str())
            .ok_or_else(|| Error::Auth("平台响应缺少 License 签名".to_string()))?;
        let license: License = serde_json::from_value(license_value.clone())
            .map_err(|e| Error::Auth(format!("License 解析失败：{e}")))?;
        if license.install_id != self.install_id {
            return Err(Error::Auth("License 的 installId 与本地不一致".to_string()));
        }
        if !license.client_pub_key.eq_ignore_ascii_case(&self.pubkey) {
            return Err(Error::Auth("License 的客户端公钥与本地不一致".to_string()));
        }
        dhrust::plugin::verify_base64(&pubkey, license.canonical().as_bytes(), signature)
            .map_err(|e| Error::Auth(format!("License 签名校验失败：{e}")))?;
        let file = LicenseFile {
            license: license.clone(),
            signature: signature.to_string(),
            pubkey,
        };
        let text = serde_json::to_vec_pretty(&file)
            .map_err(|e| Error::Auth(format!("License 序列化失败：{e}")))?;
        write_private(&self.license_path, &text)?;
        self.license = Some(license);
        Ok(())
    }

    fn license_valid(&self) -> bool {
        let Some(license) = &self.license else {
            return false;
        };
        if license.install_id != self.install_id
            || !license.client_pub_key.eq_ignore_ascii_case(&self.pubkey)
        {
            return false;
        }
        license.not_after > now_secs() + 60
    }
}

/// 管理器持有的授权运行时（含“平台不支持”降级标记）。
#[derive(Debug)]
pub(crate) struct AuthRuntime {
    /// 客户端身份与 License
    pub client: AuthClient,
    /// 平台未启用授权接口（自动回退公开下载，避免旧平台不可用）
    pub unsupported: bool,
}

/// 读取 License 文件（无效/损坏返回 `Ok(None)`）。
fn read_license(path: &Path) -> Result<Option<License>> {
    let text = match fs::read_to_string(path) {
        Ok(text) => text,
        Err(_) => return Ok(None),
    };
    let file: LicenseFile = match serde_json::from_str(&text) {
        Ok(file) => file,
        Err(_) => return Ok(None),
    };
    // 验签（配置公钥可能尚未加载；此处只做结构解析，真正验签在 apply 时）
    Ok(Some(file.license))
}

fn valid_install_id(s: &str) -> bool {
    let n = s.chars().count();
    (8..=64).contains(&n)
        && s.chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-'))
}

fn now_secs() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

fn hostname() -> String {
    dhrust::sys::machine::hostname()
}

fn machine_id() -> String {
    // 仅审计用：dhrust 各平台可用的机器指纹（取不到则留空）
    dhrust::sys::machine::machine_guid().unwrap_or_default()
}

fn validate_ticket_url(url: &str, origin: &str) -> Result<()> {
    let ok_scheme = url.starts_with("https://")
        || (url.starts_with("http://")
            && (origin.starts_with("http://127.0.0.1")
                || origin.starts_with("http://localhost")
                || origin.starts_with("http://[::1]")));
    if !ok_scheme {
        return Err(Error::Auth(format!(
            "平台返回的下载地址不是 HTTPS（{url}）"
        )));
    }
    let origin = origin.trim_end_matches('/');
    if !url.starts_with(&format!("{origin}/components/")) {
        return Err(Error::Auth(format!(
            "平台返回的下载地址与组件源不一致（{url}）"
        )));
    }
    Ok(())
}

fn write_private(path: &Path, data: &[u8]) -> Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    fs::write(path, data)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = fs::set_permissions(path, fs::Permissions::from_mode(0o600));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn license_canonical_matches_platform() {
        let license = License {
            license_id: "lic_x".into(),
            install_id: "x".into(),
            client_pub_key: "k".into(),
            datasets: vec!["a".into(), "b".into()],
            auth_mode: "auto".into(),
            issued_at: 1,
            not_after: 2,
            daily_quota: 3,
        };
        assert_eq!(
            license.canonical(),
            "pek-ip-license-v1\nlic_x\nx\nk\na,b\nauto\n1\n2\n3"
        );
    }

    #[test]
    fn ticket_url_validation() {
        assert!(
            validate_ticket_url(
                "https://p.sc8.fun/components/a.zip?ticket=x",
                "https://p.sc8.fun"
            )
            .is_ok()
        );
        assert!(validate_ticket_url("https://evil.example/a.zip", "https://p.sc8.fun").is_err());
        assert!(
            validate_ticket_url(
                "https://p.sc8.fun.evil.example/components/a.zip",
                "https://p.sc8.fun"
            )
            .is_err()
        );
        assert!(
            validate_ticket_url("http://p.sc8.fun/components/a.zip", "http://p.sc8.fun").is_err()
        );
        assert!(
            validate_ticket_url(
                "http://127.0.0.1:5502/components/a.zip",
                "http://127.0.0.1:5502"
            )
            .is_ok()
        );
    }
}
