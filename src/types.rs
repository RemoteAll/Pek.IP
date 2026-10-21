//! 公共结果类型：统一 IP 定位结果、特殊地址分类、国外数据记录。

use std::net::{IpAddr, Ipv4Addr};

/// 数据来源。
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
#[cfg_attr(
    feature = "serde",
    derive(serde::Serialize, serde::Deserialize),
    serde(rename_all = "snake_case")
)]
pub enum DataSource {
    /// 内网/保留/组播等特殊地址，未查询数据库。
    Reserved,
    /// 仅命中 ip2region（国内数据）。
    Ip2Region,
    /// 仅命中 MMDB（国外数据）。
    Mmdb,
    /// ip2region + MMDB 合并（国内城市用 xdb，经纬度/时区等用 MMDB）。
    Merged,
    /// 数据库中没有该地址的记录。
    #[default]
    Unknown,
}

impl DataSource {
    /// 中文标签。
    pub fn label(self) -> &'static str {
        match self {
            Self::Reserved => "保留地址",
            Self::Ip2Region => "ip2region",
            Self::Mmdb => "MMDB",
            Self::Merged => "ip2region + MMDB",
            Self::Unknown => "未知",
        }
    }
}

/// 特殊 IPv4/IPv6 地址分类。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[cfg_attr(
    feature = "serde",
    derive(serde::Serialize, serde::Deserialize),
    serde(rename_all = "snake_case")
)]
pub enum IpClass {
    /// 未指定地址（`0.0.0.0` / `::`）
    Unspecified,
    /// 回环地址（`127.0.0.0/8` / `::1`）
    Loopback,
    /// 私有地址（RFC1918 / `fc00::/7`）
    Private,
    /// 运营商级 NAT（`100.64.0.0/10`）
    Cgnat,
    /// 链路本地（`169.254.0.0/16` / `fe80::/10`）
    LinkLocal,
    /// 组播（`224.0.0.0/4` / `ff00::/8`）
    Multicast,
    /// 文档示例地址（TEST-NET / `2001:db8::/32`）
    Documentation,
    /// 基准测试地址（`198.18.0.0/15`）
    Benchmark,
    /// 共享地址（`100::/64`）
    Shared,
    /// 其它保留地址（`0.0.0.0/8`、`192.0.0.0/24`、`240.0.0.0/4` 等）
    Reserved,
    /// NAT64 地址（`64:ff9b::/96`）
    Nat64,
    /// ORCHID 地址（`2001:10::/28`）
    Orchid,
}

impl IpClass {
    /// 中文标签（可直接作为“国家/地区”展示）。
    pub fn label(self) -> &'static str {
        match self {
            Self::Unspecified => "未指定地址",
            Self::Loopback => "回环地址",
            Self::Private => "局域网",
            Self::Cgnat => "运营商级 NAT",
            Self::LinkLocal => "链路本地",
            Self::Multicast => "组播地址",
            Self::Documentation => "文档示例地址",
            Self::Benchmark => "基准测试地址",
            Self::Shared => "共享地址",
            Self::Reserved => "保留地址",
            Self::Nat64 => "NAT64 地址",
            Self::Orchid => "ORCHID 地址",
        }
    }
}

/// 国外数据（MMDB）的一条城市记录。
#[derive(Clone, Debug, Default, PartialEq)]
pub struct WorldRecord {
    /// ISO 3166-1 alpha-2（如 `US`）
    pub country_code: Option<String>,
    /// 简体中文国家名（MMDB 自带或按 ISO 翻译）
    pub country_zh: Option<String>,
    /// 英文国家名
    pub country_en: Option<String>,
    /// 洲代码（如 `NA`）
    pub continent_code: Option<String>,
    /// 洲中文名
    pub continent_zh: Option<String>,
    /// 省/州
    pub province: Option<String>,
    /// 城市
    pub city: Option<String>,
    /// 区县（MMDB subdivisions 的第二级）
    pub district: Option<String>,
    /// 纬度
    pub latitude: Option<f64>,
    /// 经度
    pub longitude: Option<f64>,
    /// IANA 时区（如 `Asia/Shanghai`）
    pub timezone: Option<String>,
}

/// 国外数据（MMDB ASN/ISP）的一条记录。
#[derive(Clone, Debug, Default, PartialEq)]
pub struct AsnRecord {
    /// 自治系统号
    pub asn: Option<u32>,
    /// 自治系统组织名
    pub organization: Option<String>,
    /// ISP 名称（部分数据库提供）
    pub isp: Option<String>,
}

/// 统一的 IP 定位结果。
///
/// 字段全部为可选：数据源缺失或该 IP 无记录时为 `None`。
/// `country` 为展示用名称（优先简体中文），`country_code` 为 ISO 代码。
#[derive(Clone, Debug, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct IpLocation {
    /// 查询的 IP（IPv4-mapped IPv6 已归一为 IPv4）
    pub ip: IpAddr,
    /// 国家/地区名称（简体中文优先）
    pub country: Option<String>,
    /// ISO 3166-1 alpha-2 国家/地区代码
    pub country_code: Option<String>,
    /// 英文国家/地区名称
    pub country_en: Option<String>,
    /// 洲中文名
    pub continent: Option<String>,
    /// 洲代码
    pub continent_code: Option<String>,
    /// 省/州
    pub province: Option<String>,
    /// 城市
    pub city: Option<String>,
    /// 区县
    pub district: Option<String>,
    /// 运营商/ISP
    pub isp: Option<String>,
    /// 自治系统号
    pub asn: Option<u32>,
    /// 自治系统组织名
    pub as_org: Option<String>,
    /// 纬度
    pub latitude: Option<f64>,
    /// 经度
    pub longitude: Option<f64>,
    /// IANA 时区
    pub timezone: Option<String>,
    /// 是否中国大陆（`CN` 或名称为“中国”）
    pub is_china: bool,
    /// 是否为非公网地址（内网/回环/保留/组播等）
    pub is_private: bool,
    /// 特殊地址分类
    pub ip_class: Option<IpClass>,
    /// 数据来源
    pub source: DataSource,
}

impl Default for IpLocation {
    fn default() -> Self {
        Self {
            ip: IpAddr::V4(Ipv4Addr::UNSPECIFIED),
            country: None,
            country_code: None,
            country_en: None,
            continent: None,
            continent_code: None,
            province: None,
            city: None,
            district: None,
            isp: None,
            asn: None,
            as_org: None,
            latitude: None,
            longitude: None,
            timezone: None,
            is_china: false,
            is_private: false,
            ip_class: None,
            source: DataSource::Unknown,
        }
    }
}

impl IpLocation {
    /// 空结果（未命中任何数据库）。
    pub fn empty(ip: IpAddr) -> Self {
        Self {
            ip,
            source: DataSource::Unknown,
            ..Default::default()
        }
    }

    /// 特殊地址结果。
    pub fn reserved(ip: IpAddr, class: IpClass) -> Self {
        Self {
            ip,
            country: Some(class.label().to_string()),
            is_private: true,
            ip_class: Some(class),
            source: DataSource::Reserved,
            ..Default::default()
        }
    }

    /// 是否命中任意一个数据源（含特殊地址分类）。
    pub fn is_known(&self) -> bool {
        self.source != DataSource::Unknown
    }

    /// 是否为公网地址（非特殊地址）。
    pub fn is_public(&self) -> bool {
        !self.is_private
    }

    /// 是否属于大中华区（`CN`/`HK`/`MO`/`TW`，或名称以“中国”开头）。
    ///
    /// 与 [`IpLocation::is_china`] 的区别：`is_china` 只认 `CN` / “中国”。
    pub fn is_china_region(&self) -> bool {
        if self.is_china {
            return true;
        }
        matches!(
            self.country_code
                .as_deref()
                .map(str::to_ascii_uppercase)
                .as_deref(),
            Some("HK" | "MO" | "TW")
        ) || self
            .country
            .as_deref()
            .is_some_and(|c| c.starts_with("中国"))
    }

    /// 展示用单行文本（如“中国 广东省 深圳市 中国电信”）。
    pub fn display_short(&self) -> String {
        let mut parts = Vec::new();
        if let Some(country) = &self.country {
            parts.push(country.as_str());
        }
        for item in [&self.province, &self.city, &self.district, &self.isp] {
            if let Some(text) = item
                && Some(text.as_str()) != self.country.as_deref()
            {
                parts.push(text.as_str());
            }
        }
        if parts.is_empty() {
            "未知".to_string()
        } else {
            parts.join(" ")
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reserved_result_has_label_and_class() {
        let loc = IpLocation::reserved("10.1.2.3".parse().unwrap(), IpClass::Private);
        assert!(loc.is_private);
        assert!(!loc.is_public());
        assert_eq!(loc.country.as_deref(), Some("局域网"));
        assert_eq!(loc.ip_class, Some(IpClass::Private));
        assert_eq!(loc.source, DataSource::Reserved);
    }

    #[test]
    fn china_region_covers_hk_mo_tw() {
        let mut loc = IpLocation::empty("1.1.1.1".parse().unwrap());
        loc.country_code = Some("HK".into());
        assert!(loc.is_china_region());
        assert!(!loc.is_china);

        loc.country_code = Some("CN".into());
        loc.is_china = true;
        assert!(loc.is_china_region());
    }

    #[test]
    fn display_short_skips_empty() {
        let mut loc = IpLocation::empty("1.1.1.1".parse().unwrap());
        loc.country = Some("中国".into());
        loc.province = Some("广东省".into());
        loc.city = Some("深圳市".into());
        loc.isp = Some("中国电信".into());
        assert_eq!(loc.display_short(), "中国 广东省 深圳市 中国电信");
    }
}
