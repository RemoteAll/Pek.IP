//! 统一 IP 定位器：国内 ip2region + 国外 MMDB 合并。
//!
//! 合并策略：
//!
//! 1. 先识别内网/回环/保留地址，直接返回，不查库；
//! 2. 国内 IP（ip2region 判定为中国）使用 ip2region 的省/市/运营商，
//!    经纬度、时区、区县等从 MMDB 补齐；
//! 3. 国外 IP 优先使用 MMDB 的城市/国家信息；
//! 4. MMDB 缺失时回退到 ip2region 的国外国家/省/市；
//! 5. 最后合并 ASN/ISP 信息。

use std::net::{IpAddr, Ipv4Addr, Ipv6Addr};
use std::path::Path;

use crate::country::{continent_name_zh, country_name_zh};
use crate::region::CnRegion;
use crate::reserved;
use crate::types::{AsnRecord, DataSource, IpLocation, WorldRecord};
use crate::xdb::XdbReader;
use crate::{Error, Result};

#[cfg(feature = "mmdb")]
use crate::mmdb::{MmdbKind, MmdbReader};

/// 统一 IP 定位器。
#[derive(Debug, Default)]
pub struct IpLocator {
    pub(crate) xdb_v4: Option<XdbReader>,
    pub(crate) xdb_v6: Option<XdbReader>,
    #[cfg(feature = "mmdb")]
    pub(crate) world: Option<MmdbReader>,
    #[cfg(feature = "mmdb")]
    pub(crate) world_asn: Option<MmdbReader>,
}

impl IpLocator {
    /// 创建构建器。
    pub fn builder() -> IpLocatorBuilder {
        IpLocatorBuilder::default()
    }

    /// 空定位器（所有公网 IP 都返回未知；仍会识别内网/保留地址）。
    pub fn empty() -> Self {
        Self::default()
    }

    /// 是否有国内 ip2region 数据。
    pub fn has_china_db(&self) -> bool {
        self.xdb_v4.is_some() || self.xdb_v6.is_some()
    }

    /// 是否有国外 MMDB 数据。
    pub fn has_world_db(&self) -> bool {
        #[cfg(feature = "mmdb")]
        {
            self.world.is_some() || self.world_asn.is_some()
        }
        #[cfg(not(feature = "mmdb"))]
        {
            false
        }
    }

    /// 查询 IP。
    pub fn lookup(&self, ip: IpAddr) -> Result<IpLocation> {
        let ip = reserved::normalize_ip(ip);
        if let Some(class) = reserved::classify(ip) {
            return Ok(IpLocation::reserved(ip, class));
        }

        let cn = self.lookup_cn(ip)?;
        #[cfg(feature = "mmdb")]
        let world = match &self.world {
            Some(reader) => reader.lookup_world(ip)?,
            None => None,
        };
        #[cfg(not(feature = "mmdb"))]
        let world: Option<WorldRecord> = None;
        #[cfg(feature = "mmdb")]
        let asn = match &self.world_asn {
            Some(reader) => reader.lookup_asn(ip)?,
            None => None,
        };
        #[cfg(not(feature = "mmdb"))]
        let asn: Option<AsnRecord> = None;

        Ok(merge_records(ip, cn, world, asn))
    }

    /// 查询 IP 文本。
    pub fn lookup_str(&self, ip: &str) -> Result<IpLocation> {
        let parsed: IpAddr = ip
            .trim()
            .parse()
            .map_err(|_| Error::InvalidIp(ip.to_string()))?;
        self.lookup(parsed)
    }

    /// 查询 IPv4 数值。
    pub fn lookup_ipv4(&self, ip: u32) -> Result<IpLocation> {
        self.lookup(IpAddr::V4(Ipv4Addr::from(ip)))
    }

    /// 查询 IPv6 数值。
    pub fn lookup_ipv6(&self, ip: u128) -> Result<IpLocation> {
        self.lookup(IpAddr::V6(Ipv6Addr::from(ip)))
    }

    fn lookup_cn(&self, ip: IpAddr) -> Result<Option<CnRegion>> {
        let candidates: [&Option<XdbReader>; 2] = match ip {
            IpAddr::V4(_) => [&self.xdb_v4, &self.xdb_v6],
            IpAddr::V6(_) => [&self.xdb_v6, &self.xdb_v4],
        };
        for reader in candidates {
            let Some(reader) = reader.as_ref() else {
                continue;
            };
            let Some(text) = reader.lookup(ip)? else {
                continue;
            };
            let region = CnRegion::parse(&text);
            if region.reserved || region.is_empty() {
                continue;
            }
            return Ok(Some(region));
        }
        Ok(None)
    }
}

/// 定位器构建器。
#[derive(Debug, Default)]
pub struct IpLocatorBuilder {
    xdb_v4: Option<XdbReader>,
    xdb_v6: Option<XdbReader>,
    #[cfg(feature = "mmdb")]
    world: Option<MmdbReader>,
    #[cfg(feature = "mmdb")]
    world_asn: Option<MmdbReader>,
}

impl IpLocatorBuilder {
    /// 新建。
    pub fn new() -> Self {
        Self::default()
    }

    /// 加载 IPv4 ip2region xdb。
    pub fn xdb_v4(mut self, path: impl AsRef<Path>) -> Result<Self> {
        let reader = XdbReader::open(path)?;
        if reader.ip_version() != crate::xdb::XdbIpVersion::V4 {
            return Err(Error::Argument(format!(
                "xdb_v4 需要 IPv4 数据库，实际为 {}",
                reader.ip_version().as_str()
            )));
        }
        self.xdb_v4 = Some(reader);
        Ok(self)
    }

    /// 直接使用已加载的 IPv4 xdb。
    pub fn xdb_v4_reader(mut self, reader: XdbReader) -> Result<Self> {
        if reader.ip_version() != crate::xdb::XdbIpVersion::V4 {
            return Err(Error::Argument(format!(
                "xdb_v4 需要 IPv4 数据库，实际为 {}",
                reader.ip_version().as_str()
            )));
        }
        self.xdb_v4 = Some(reader);
        Ok(self)
    }

    /// 加载 IPv6 ip2region xdb。
    pub fn xdb_v6(mut self, path: impl AsRef<Path>) -> Result<Self> {
        let reader = XdbReader::open(path)?;
        if reader.ip_version() != crate::xdb::XdbIpVersion::V6 {
            return Err(Error::Argument(format!(
                "xdb_v6 需要 IPv6 数据库，实际为 {}",
                reader.ip_version().as_str()
            )));
        }
        self.xdb_v6 = Some(reader);
        Ok(self)
    }

    /// 直接使用已加载的 IPv6 xdb。
    pub fn xdb_v6_reader(mut self, reader: XdbReader) -> Result<Self> {
        if reader.ip_version() != crate::xdb::XdbIpVersion::V6 {
            return Err(Error::Argument(format!(
                "xdb_v6 需要 IPv6 数据库，实际为 {}",
                reader.ip_version().as_str()
            )));
        }
        self.xdb_v6 = Some(reader);
        Ok(self)
    }

    /// 加载国外城市库（GeoLite2-City / DB-IP City / GeoLite2-Country）。
    #[cfg(feature = "mmdb")]
    pub fn world(mut self, path: impl AsRef<Path>) -> Result<Self> {
        let reader = MmdbReader::open(path, MmdbKind::Auto)?;
        if !reader.kind().is_city_like() && reader.kind() != MmdbKind::Country {
            return Err(Error::Argument(format!(
                "world 需要城市/国家库，实际为 {:?}（ASN 请用 world_asn）",
                reader.kind()
            )));
        }
        self.world = Some(reader);
        Ok(self)
    }

    /// 直接使用已加载的国外城市库。
    #[cfg(feature = "mmdb")]
    pub fn world_reader(mut self, reader: MmdbReader) -> Result<Self> {
        if !reader.kind().is_city_like() && reader.kind() != MmdbKind::Country {
            return Err(Error::Argument(format!(
                "world 需要城市/国家库，实际为 {:?}（ASN 请用 world_asn）",
                reader.kind()
            )));
        }
        self.world = Some(reader);
        Ok(self)
    }

    /// 加载国外 ASN/ISP 库。
    #[cfg(feature = "mmdb")]
    pub fn world_asn(mut self, path: impl AsRef<Path>) -> Result<Self> {
        let reader = MmdbReader::open(path, MmdbKind::Auto)?;
        if !matches!(reader.kind(), MmdbKind::Asn | MmdbKind::Isp) {
            return Err(Error::Argument(format!(
                "world_asn 需要 ASN/ISP 库，实际为 {:?}",
                reader.kind()
            )));
        }
        self.world_asn = Some(reader);
        Ok(self)
    }

    /// 直接使用已加载的 ASN/ISP 库。
    #[cfg(feature = "mmdb")]
    pub fn world_asn_reader(mut self, reader: MmdbReader) -> Result<Self> {
        if !matches!(reader.kind(), MmdbKind::Asn | MmdbKind::Isp) {
            return Err(Error::Argument(format!(
                "world_asn 需要 ASN/ISP 库，实际为 {:?}",
                reader.kind()
            )));
        }
        self.world_asn = Some(reader);
        Ok(self)
    }

    /// 构建定位器。
    pub fn build(self) -> IpLocator {
        IpLocator {
            xdb_v4: self.xdb_v4,
            xdb_v6: self.xdb_v6,
            #[cfg(feature = "mmdb")]
            world: self.world,
            #[cfg(feature = "mmdb")]
            world_asn: self.world_asn,
        }
    }
}

/// 合并国内/国外记录（公开给同 crate 测试）。
pub(crate) fn merge_records(
    ip: IpAddr,
    cn: Option<CnRegion>,
    world: Option<WorldRecord>,
    asn: Option<AsnRecord>,
) -> IpLocation {
    let mut out = IpLocation::empty(ip);

    if let Some(w) = world {
        out.country_code = w.country_code;
        out.country = w.country_zh.or_else(|| {
            out.country_code
                .as_deref()
                .and_then(country_name_zh)
                .map(str::to_string)
        });
        out.country_en = w.country_en;
        out.continent_code = w.continent_code;
        out.continent = w.continent_zh.or_else(|| {
            out.continent_code
                .as_deref()
                .and_then(continent_name_zh)
                .map(str::to_string)
        });
        out.province = w.province;
        out.city = w.city;
        out.district = w.district;
        out.latitude = w.latitude;
        out.longitude = w.longitude;
        out.timezone = w.timezone;
        out.source = DataSource::Mmdb;
    }

    if let Some(mut c) = cn {
        let had_world = out.source == DataSource::Mmdb;
        if c.is_china {
            out.is_china = true;
            out.country = Some(c.country_display_zh().unwrap_or_else(|| "中国".to_string()));
            out.country_code = Some(c.country_code.clone().unwrap_or_else(|| "CN".to_string()));
            out.country_en = Some("China".to_string());
            out.continent = Some("亚洲".to_string());
            out.continent_code = Some("AS".to_string());

            let province_changed =
                c.province.is_some() && out.province.as_deref() != c.province.as_deref();
            if province_changed {
                out.district = None;
            }
            if c.province.is_some() {
                out.province = c.province;
            }
            let city_changed = c.city.is_some() && out.city.as_deref() != c.city.as_deref();
            if city_changed {
                out.district = None;
            }
            if c.city.is_some() {
                out.city = c.city;
            }
            if c.isp.is_some() {
                out.isp = c.isp;
            }
            out.source = if had_world {
                DataSource::Merged
            } else {
                DataSource::Ip2Region
            };
        } else if out.country.is_none() && !c.is_empty() {
            out.country = c.country_display_zh();
            out.country_code = c.country_code;
            out.province = c.province;
            out.city = c.city;
            out.isp = c.isp;
            out.source = if had_world {
                DataSource::Merged
            } else {
                DataSource::Ip2Region
            };
        } else if had_world {
            // 国外：MMDB 为主；ip2region 只补齐缺失字段，未使用则不标记 Merged
            let mut used_cn = false;
            if out.province.is_none()
                && let Some(value) = c.province.take()
            {
                out.province = Some(value);
                used_cn = true;
            }
            if out.city.is_none()
                && let Some(value) = c.city.take()
            {
                out.city = Some(value);
                used_cn = true;
            }
            if out.isp.is_none()
                && let Some(value) = c.isp.take()
            {
                out.isp = Some(value);
                used_cn = true;
            }
            if used_cn {
                out.source = DataSource::Merged;
            }
        }
    }

    if let Some(a) = asn {
        let AsnRecord {
            asn,
            organization,
            isp,
        } = a;
        if out.asn.is_none() {
            out.asn = asn;
        }
        if out.isp.is_none() {
            out.isp = isp.or_else(|| organization.clone());
        }
        if out.as_org.is_none() {
            out.as_org = organization;
        }
    }

    if out.country.is_none()
        && let Some(code) = out.country_code.as_deref()
    {
        out.country = country_name_zh(code).map(str::to_string);
    }
    if out.country_code.as_deref() == Some("CN") || out.country.as_deref() == Some("中国") {
        out.is_china = true;
        if out.country.is_none() {
            out.country = Some("中国".to_string());
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn world_us() -> WorldRecord {
        WorldRecord {
            country_code: Some("US".into()),
            country_zh: Some("美国".into()),
            country_en: Some("United States".into()),
            continent_code: Some("NA".into()),
            continent_zh: Some("北美洲".into()),
            province: Some("California".into()),
            city: Some("Los Angeles".into()),
            latitude: Some(34.05),
            longitude: Some(-118.24),
            timezone: Some("America/Los_Angeles".into()),
            ..Default::default()
        }
    }

    #[test]
    fn china_record_wins_city_but_keeps_world_coordinates() {
        let cn = CnRegion::parse("中国|广东省|深圳市|中国电信|CN");
        let loc = merge_records("1.2.3.4".parse().unwrap(), Some(cn), Some(world_us()), None);
        assert!(loc.is_china);
        assert_eq!(loc.country.as_deref(), Some("中国"));
        assert_eq!(loc.country_code.as_deref(), Some("CN"));
        assert_eq!(loc.province.as_deref(), Some("广东省"));
        assert_eq!(loc.city.as_deref(), Some("深圳市"));
        assert_eq!(loc.isp.as_deref(), Some("中国电信"));
        assert_eq!(loc.latitude, Some(34.05));
        assert_eq!(loc.timezone.as_deref(), Some("America/Los_Angeles"));
        assert_eq!(loc.source, DataSource::Merged);
    }

    #[test]
    fn world_only_record() {
        let loc = merge_records(
            "8.8.8.8".parse().unwrap(),
            None,
            Some(world_us()),
            Some(AsnRecord {
                asn: Some(15169),
                organization: Some("Google LLC".into()),
                isp: None,
            }),
        );
        assert_eq!(loc.country.as_deref(), Some("美国"));
        assert_eq!(loc.country_code.as_deref(), Some("US"));
        assert_eq!(loc.province.as_deref(), Some("California"));
        assert_eq!(loc.city.as_deref(), Some("Los Angeles"));
        assert_eq!(loc.asn, Some(15169));
        assert_eq!(loc.isp.as_deref(), Some("Google LLC"));
        assert_eq!(loc.source, DataSource::Mmdb);
    }

    #[test]
    fn china_only_record() {
        let cn = CnRegion::parse("中国|福建省|福州市|中国电信|CN");
        let loc = merge_records("1.2.3.4".parse().unwrap(), Some(cn), None, None);
        assert!(loc.is_china);
        assert_eq!(loc.country.as_deref(), Some("中国"));
        assert_eq!(loc.province.as_deref(), Some("福建省"));
        assert_eq!(loc.city.as_deref(), Some("福州市"));
        assert_eq!(loc.source, DataSource::Ip2Region);
        assert_eq!(loc.latitude, None);
    }

    #[test]
    fn overseas_from_xdb_when_world_missing() {
        let cn = CnRegion::parse("Australia|Queensland|Brisbane|0|AU");
        let loc = merge_records("1.0.0.1".parse().unwrap(), Some(cn), None, None);
        assert!(!loc.is_china);
        assert_eq!(loc.country.as_deref(), Some("澳大利亚"));
        assert_eq!(loc.country_code.as_deref(), Some("AU"));
        assert_eq!(loc.province.as_deref(), Some("Queensland"));
        assert_eq!(loc.city.as_deref(), Some("Brisbane"));
        assert_eq!(loc.source, DataSource::Ip2Region);
    }

    #[test]
    fn world_primary_keeps_mmdb_source_when_cn_adds_nothing() {
        let cn = CnRegion::parse("Sweden|0|0|0|SE");
        let loc = merge_records(
            "89.160.20.128".parse().unwrap(),
            Some(cn),
            Some(world_us()),
            None,
        );
        assert_eq!(loc.country_code.as_deref(), Some("US"));
        assert_eq!(loc.source, DataSource::Mmdb);
    }

    #[test]
    fn cn_isp_fills_world_record_marks_merged() {
        let cn = CnRegion::parse("United States|Texas|0|Some ISP|US");
        let loc = merge_records("8.8.8.8".parse().unwrap(), Some(cn), Some(world_us()), None);
        assert_eq!(loc.isp.as_deref(), Some("Some ISP"));
        assert_eq!(loc.source, DataSource::Merged);
    }

    #[test]
    fn isp_falls_back_to_asn_organization() {
        let loc = merge_records(
            "8.8.8.8".parse().unwrap(),
            None,
            None,
            Some(AsnRecord {
                asn: Some(15169),
                organization: Some("Google LLC".into()),
                isp: None,
            }),
        );
        assert_eq!(loc.isp.as_deref(), Some("Google LLC"));
        assert_eq!(loc.as_org.as_deref(), Some("Google LLC"));
    }

    #[test]
    fn reserved_ip_short_circuits() {
        let locator = IpLocator::empty();
        let loc = locator.lookup_str("192.168.1.1").unwrap();
        assert!(loc.is_private);
        assert_eq!(loc.source, DataSource::Reserved);
        assert_eq!(loc.ip_class, Some(crate::types::IpClass::Private));
    }

    #[test]
    fn invalid_ip_text() {
        let locator = IpLocator::empty();
        let err = locator.lookup_str("not-an-ip").unwrap_err();
        assert!(matches!(err, Error::InvalidIp(_)));
    }

    #[test]
    fn empty_locator_returns_unknown() {
        let loc = IpLocator::empty().lookup_str("8.8.8.8").unwrap();
        assert_eq!(loc.source, DataSource::Unknown);
        assert!(!loc.is_known());
    }
}
