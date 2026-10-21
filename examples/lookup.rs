//! 本地数据查询示例：
//!
//! ```text
//! set IP2REGION_V4=data\ip2region_v4.xdb
//! set GEOLITE2_CITY=data\GeoLite2-City.mmdb
//! set GEOLITE2_ASN=data\GeoLite2-ASN.mmdb
//! cargo run --example lookup -- 114.114.114.114 8.8.8.8 240e:3b7:3273:51d0::1 192.168.1.1
//! ```

use pek_ip::IpLocator;

fn main() -> Result<(), pek_ip::Error> {
    let mut builder = IpLocator::builder();

    if let Ok(path) = std::env::var("IP2REGION_V4") {
        builder = builder.xdb_v4(path)?;
    }
    if let Ok(path) = std::env::var("IP2REGION_V6") {
        builder = builder.xdb_v6(path)?;
    }
    #[cfg(feature = "mmdb")]
    {
        if let Ok(path) = std::env::var("GEOLITE2_CITY") {
            builder = builder.world(path)?;
        }
        if let Ok(path) = std::env::var("GEOLITE2_ASN") {
            builder = builder.world_asn(path)?;
        }
    }

    let locator = builder.build();
    if !locator.has_china_db() {
        eprintln!("提示：未设置 IP2REGION_V4 / IP2REGION_V6，国内数据不可用");
    }
    if !locator.has_world_db() {
        eprintln!("提示：未设置 GEOLITE2_CITY / GEOLITE2_ASN，国外数据不可用");
    }

    let mut ips: Vec<String> = std::env::args().skip(1).collect();
    if ips.is_empty() {
        ips = vec![
            "114.114.114.114".to_string(),
            "8.8.8.8".to_string(),
            "240e:3b7:3273:51d0::1".to_string(),
            "192.168.1.1".to_string(),
        ];
    }
    for ip in ips {
        match locator.lookup_str(&ip) {
            Ok(loc) => {
                println!("{ip} => {}", loc.display_short());
                println!("{loc:#?}");
            }
            Err(err) => eprintln!("{ip} => 错误：{err}"),
        }
    }
    Ok(())
}
