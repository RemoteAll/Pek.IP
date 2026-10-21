//! # pek-ip —— 国内 + 国外离线 IP 定位库
//!
//! `pek-ip` 面向服务端场景，提供统一的 IPv4/IPv6 定位结果：
//!
//! - **国内**：读取 ip2region xdb v3（城市/省/运营商，中文数据）；
//! - **国外**：读取 MaxMind GeoLite2/GeoIP2、DB-IP 等 MMDB（国家/省/市/经纬度/时区/ASN）；
//! - **合并**：国内 IP 以 ip2region 的省市为准，经纬度/时区/区县由 MMDB 补齐；
//! - **特殊地址**：内网、回环、链路本地、组播、文档/保留地址直接识别，不查库；
//! - **按需下载**（`manager` 特性）：从 Pek.RPanlServer 组件源下载数据包，
//!   Ed25519 验签 + SHA-256 校验，本地缓存，离线可用。
//!
//! 数据文件不随库分发，请按各数据源的许可自行获取/上传。
//!
//! ## 快速开始（本地数据）
//!
//! ```no_run
//! use pek_ip::IpLocator;
//!
//! # fn main() -> Result<(), pek_ip::Error> {
//! let locator = IpLocator::builder()
//!     .xdb_v4("data/ip2region_v4.xdb")?
//!     .build();
//!
//! let loc = locator.lookup_str("114.114.114.114")?;
//! println!("{}（{}）", loc.display_short(), loc.source.label());
//! # Ok(())
//! # }
//! ```
//!
//! 启用默认特性后还可以加载 MMDB：
//!
//! ```ignore
//! let locator = IpLocator::builder()
//!     .xdb_v4("data/ip2region_v4.xdb")?
//!     .xdb_v6("data/ip2region_v6.xdb")?
//!     .world("data/GeoLite2-City.mmdb")?
//!     .world_asn("data/GeoLite2-ASN.mmdb")?
//!     .build();
//! ```
//!
//! ## 按需下载
//!
//! ```ignore
//! use pek_ip::{IpDbManager, IpDbManagerConfig, DATASET_IP2REGION_V4};
//!
//! let mgr = IpDbManager::new(IpDbManagerConfig {
//!     store_url: "https://p.sc8.fun".into(),
//!     pubkey: "<平台 Ed25519 公钥 hex>".into(),
//!     ..Default::default()
//! })?;
//! let db = mgr.ensure_updated(DATASET_IP2REGION_V4)?;
//! let locator = pek_ip::IpLocator::builder()
//!     .xdb_v4(db.file_with_ext("xdb").unwrap())?
//!     .build();
//! ```
//!
//! 详见 README 与 `examples/`。

pub mod country;
pub mod error;
pub mod locator;
pub mod region;
pub mod reserved;
pub mod types;
pub mod xdb;

#[cfg(feature = "mmdb")]
pub mod mmdb;

#[cfg(feature = "manager")]
pub mod manager;

#[cfg(feature = "manager")]
pub mod config;

pub use error::{Error, Result};
pub use locator::{IpLocator, IpLocatorBuilder};
pub use region::CnRegion;
pub use types::{AsnRecord, DataSource, IpClass, IpLocation, WorldRecord};
pub use xdb::{XdbIndexPolicy, XdbIpVersion, XdbReader};

#[cfg(feature = "mmdb")]
pub use mmdb::{MmdbKind, MmdbReader};

#[cfg(feature = "manager")]
pub use config::{IpConfig, DEFAULT_STORE_URL};

#[cfg(feature = "manager")]
pub use manager::{
    ComponentEntry, DATASET_DBIP_CITY, DATASET_GEOLITE2_ASN, DATASET_GEOLITE2_CITY,
    DATASET_IP2REGION_V4, DATASET_IP2REGION_V6, IP_DATABASE_CATEGORY, InstalledDb, IpDbManager,
    IpDbManagerConfig, LocatorLoadOptions,
};
