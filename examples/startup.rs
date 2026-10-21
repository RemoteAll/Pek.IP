//! 启动时自动加载/下载示例：
//!
//! 1. 先查本地 `data/`（仓库自带 ip2region v4/v6、DB-IP Country/ASN）；
//! 2. 缺失时从 Pek.RPanlServer 组件源按需下载；
//! 3. 都不可用时按 `allow_partial` 决定跳过或报错。
//!
//! ```text
//! set PEK_IP_STORE_URL=https://p.sc8.fun
//! set PEK_IP_STORE_PUBKEY=<平台 Ed25519 公钥 hex>
//! set PEK_IP_DATA_DIR=data
//! cargo run --features manager --example startup -- 114.114.114.114 8.8.8.8
//! ```

use pek_ip::{IpDbManagerConfig, IpLocator, LocatorLoadOptions};

fn main() -> Result<(), pek_ip::Error> {
    let config = IpDbManagerConfig {
        store_url: std::env::var("PEK_IP_STORE_URL").unwrap_or_default(),
        pubkey: std::env::var("PEK_IP_STORE_PUBKEY").unwrap_or_default(),
        cache_dir: std::env::var_os("PEK_IP_CACHE_DIR").map(Into::into),
        access_token: std::env::var("PEK_IP_STORE_TOKEN").ok(),
        ..Default::default()
    };
    let data_dir = std::env::var_os("PEK_IP_DATA_DIR")
        .map(Into::into)
        .unwrap_or_else(|| "data".into());

    let locator = IpLocator::load_or_download(
        config,
        LocatorLoadOptions {
            data_dir,
            load_v6: true,
            download_v6: false, // 需要 IPv6 且允许下载 37MB 时改为 true
            ..Default::default()
        },
    )?;

    println!(
        "国内数据：{}，国外数据：{}",
        if locator.has_china_db() {
            "已加载"
        } else {
            "缺失"
        },
        if locator.has_world_db() {
            "已加载"
        } else {
            "缺失"
        }
    );

    let mut ips: Vec<String> = std::env::args().skip(1).collect();
    if ips.is_empty() {
        ips = vec!["114.114.114.114".into(), "8.8.8.8".into()];
    }
    for ip in ips {
        let loc = locator.lookup_str(&ip)?;
        println!("{ip} => {}", loc.display_short());
        println!("{loc:#?}");
    }
    Ok(())
}
