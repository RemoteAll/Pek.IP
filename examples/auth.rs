//! 自动授权下载示例（零身份配置）：
//!
//! ```text
//! set PEK_IP_BASE=G:\temp\pek-ip-auth-demo
//! cargo run --features manager --example auth
//! ```
//!
//! `Config/Ip.toml` 的 `AutoEnroll=true` 时，首次下载会自动生成 installId/密钥、
//! 向平台注册并申请短期票据；平台按来源 CIDR 自动批准或管理员在面板批准后即可下载。

use pek_ip::{DATASET_IP2REGION_V4, IpConfig, IpDbManager};

fn main() -> Result<(), pek_ip::Error> {
    let base = std::env::var_os("PEK_IP_BASE")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| std::env::current_dir().unwrap_or_else(|_| ".".into()));
    let cfg = IpConfig::load_or_default(&base)?;
    println!("平台地址：{}", cfg.store_url);
    println!("自动注册：{}", cfg.auto_enroll);
    println!("状态目录：{}", cfg.state_dir.display());

    let manager = IpDbManager::new(cfg.to_manager_config()?)?;
    let dataset =
        std::env::var("PEK_IP_DATASET").unwrap_or_else(|_| DATASET_IP2REGION_V4.to_string());
    let db = manager.ensure_updated(&dataset)?;
    println!(
        "已安装：{} {}（{} 个文件）",
        db.id,
        db.version,
        db.files.len()
    );
    for file in &db.files {
        println!("  {}", file.display());
    }
    println!(
        "授权状态：installId={}，License={}",
        std::fs::read_to_string(cfg.state_dir.join("install.id"))
            .unwrap_or_default()
            .trim(),
        if cfg.state_dir.join("license.json").is_file() {
            "已获取"
        } else {
            "未获取"
        }
    );
    Ok(())
}
