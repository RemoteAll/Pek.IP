//! 启动时自动加载/下载示例（零业务配置）：
//!
//! 1. 读取 `Config/Ip.toml`（平台地址固定，默认 `https://p.sc8.fun`；不存在自动生成）；
//! 2. 先查本地 `data/`（仓库自带 ip2region v4/v6、DB-IP Country/ASN）；
//! 3. 缺失时从 Pek.RPanlServer 组件源按需下载；
//! 4. 都不可用时按 `allow_partial` 决定跳过或报错。
//!
//! ```text
//! cargo run --features manager --example startup -- 114.114.114.114 8.8.8.8
//! ```
//!
//! 临时切换平台地址：改 `Config/Ip.toml` 的 `StoreUrl`，或设置 `PEK_IP_STORE_URL`。

use pek_ip::{IpConfig, IpLocator};

fn main() -> Result<(), pek_ip::Error> {
    let base = std::env::current_dir()?;
    let config = IpConfig::load_or_default(&base)?;
    println!("平台地址：{}", config.store_url);

    let locator =
        IpLocator::load_or_download(config.to_manager_config()?, config.to_load_options())?;

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
