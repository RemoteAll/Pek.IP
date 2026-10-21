//! 按需下载示例：
//!
//! ```text
//! set PEK_IP_STORE_URL=https://p.sc8.fun
//! set PEK_IP_STORE_PUBKEY=<平台 Ed25519 公钥 hex>
//! set PEK_IP_DATASET=ip2region-v4
//! cargo run --features manager --example store
//! ```

use pek_ip::{DATASET_IP2REGION_V4, IpDbManager, IpDbManagerConfig};

fn main() -> Result<(), pek_ip::Error> {
    let store_url = std::env::var("PEK_IP_STORE_URL").unwrap_or_default();
    let pubkey = std::env::var("PEK_IP_STORE_PUBKEY").unwrap_or_default();
    let cache_dir = std::env::var_os("PEK_IP_CACHE_DIR").map(Into::into);
    let dataset =
        std::env::var("PEK_IP_DATASET").unwrap_or_else(|_| DATASET_IP2REGION_V4.to_string());

    let manager = IpDbManager::new(IpDbManagerConfig {
        store_url,
        pubkey,
        cache_dir,
        ..Default::default()
    })?;

    println!("缓存目录：{}", manager.cache_dir().display());
    println!("联网模式：{}", manager.is_online());

    let db = if manager.is_online() {
        manager.ensure_updated(&dataset)?
    } else {
        manager.ensure(&dataset)?
    };
    println!("数据集：{} {}", db.id, db.version);
    println!("安装目录：{}", db.dir.display());
    for file in &db.files {
        println!("数据文件：{}", file.display());
    }
    Ok(())
}
