//! MaxMind MMDB 读取（GeoLite2 / GeoIP2 / DB-IP 等）。
//!
//! 支持常见的数据库类型：
//!
//! | 数据库 | `MmdbKind` | 用途 |
//! |--------|------------|------|
//! | GeoLite2-City / DB-IP City | [`MmdbKind::City`] | 国家/省/市/经纬度/时区 |
//! | GeoLite2-Country | [`MmdbKind::Country`] | 国家/洲 |
//! | GeoLite2-ASN | [`MmdbKind::Asn`] | ASN/组织 |
//! | GeoIP2-ISP | [`MmdbKind::Isp`] | ASN/ISP/组织 |
//!
//! `MmdbKind::Auto` 会按 `metadata.database_type` 自动识别。
//! 读取使用纯 Rust 的 `maxminddb` crate，无 C 依赖。

use std::net::IpAddr;
use std::path::{Path, PathBuf};

use maxminddb::geoip2;

use crate::country::{continent_name_zh, country_name_zh};
use crate::{AsnRecord, Error, Result, WorldRecord};

/// MMDB 数据库类型。
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum MmdbKind {
    /// 自动识别。
    #[default]
    Auto,
    /// 城市库（GeoLite2-City / GeoIP2-City / DB-IP City）。
    City,
    /// 国家库（GeoLite2-Country）。
    Country,
    /// ASN 库（GeoLite2-ASN）。
    Asn,
    /// ISP 库（GeoIP2-ISP）。
    Isp,
    /// 企业库（GeoIP2-Enterprise，按城市库字段读取）。
    Enterprise,
}

impl MmdbKind {
    /// 按 `metadata.database_type` 识别。
    pub fn from_database_type(database_type: &str) -> Self {
        let name = database_type.to_ascii_uppercase();
        if name.contains("ASN") {
            Self::Asn
        } else if name.contains("ISP") {
            Self::Isp
        } else if name.contains("COUNTRY") {
            Self::Country
        } else if name.contains("ENTERPRISE") {
            Self::Enterprise
        } else {
            Self::City
        }
    }

    /// 是否城市类库（含企业库）。
    pub fn is_city_like(self) -> bool {
        matches!(self, Self::City | Self::Enterprise)
    }
}

/// MMDB 读取器。
pub struct MmdbReader {
    inner: maxminddb::Reader<Vec<u8>>,
    kind: MmdbKind,
    database_type: String,
    path: Option<PathBuf>,
}

impl std::fmt::Debug for MmdbReader {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("MmdbReader")
            .field("path", &self.path)
            .field("kind", &self.kind)
            .field("database_type", &self.database_type)
            .finish()
    }
}

impl MmdbReader {
    /// 打开 MMDB 文件。
    pub fn open(path: impl AsRef<Path>, kind: MmdbKind) -> Result<Self> {
        let path = path.as_ref();
        if !path.is_file() {
            return Err(Error::MissingFile(path.to_path_buf()));
        }
        let data = std::fs::read(path)?;
        let mut reader = Self::from_bytes(data, kind)?;
        reader.path = Some(path.to_path_buf());
        Ok(reader)
    }

    /// 打开并自动识别数据库类型。
    pub fn open_auto(path: impl AsRef<Path>) -> Result<Self> {
        Self::open(path, MmdbKind::Auto)
    }

    /// 从字节构造。
    pub fn from_bytes(data: Vec<u8>, kind: MmdbKind) -> Result<Self> {
        let inner = maxminddb::Reader::from_source(data)?;
        let database_type = inner.metadata().database_type.clone();
        let kind = if kind == MmdbKind::Auto {
            MmdbKind::from_database_type(&database_type)
        } else {
            kind
        };
        Ok(Self {
            inner,
            kind,
            database_type,
            path: None,
        })
    }

    /// 文件路径。
    pub fn path(&self) -> Option<&Path> {
        self.path.as_deref()
    }

    /// 数据库类型。
    pub fn database_type(&self) -> &str {
        &self.database_type
    }

    /// 实际使用的读取类型（`Auto` 已解析）。
    pub fn kind(&self) -> MmdbKind {
        self.kind
    }

    /// 数据库支持的 IP 版本（4 或 6）。
    pub fn ip_version(&self) -> u16 {
        self.inner.metadata().ip_version
    }

    /// 查询城市/国家记录（仅 City/Country/Enterprise 类数据库）。
    pub fn lookup_world(&self, ip: IpAddr) -> Result<Option<WorldRecord>> {
        if !self.kind.is_city_like() && self.kind != MmdbKind::Country {
            return Ok(None);
        }
        let result = self.inner.lookup(ip)?;
        if !result.has_data() {
            return Ok(None);
        }
        if self.kind == MmdbKind::Country {
            let Some(rec) = result.decode::<geoip2::Country>()? else {
                return Ok(None);
            };
            return Ok(Some(world_from_country(&rec)));
        }
        let Some(rec) = result.decode::<geoip2::City>()? else {
            return Ok(None);
        };
        Ok(Some(world_from_city(&rec)))
    }

    /// 查询 ASN/ISP 记录（仅 Asn/Isp 类数据库）。
    pub fn lookup_asn(&self, ip: IpAddr) -> Result<Option<AsnRecord>> {
        match self.kind {
            MmdbKind::Asn => {
                let result = self.inner.lookup(ip)?;
                if !result.has_data() {
                    return Ok(None);
                }
                let Some(rec) = result.decode::<geoip2::Asn>()? else {
                    return Ok(None);
                };
                Ok(Some(AsnRecord {
                    asn: rec.autonomous_system_number,
                    organization: rec.autonomous_system_organization.map(str::to_string),
                    isp: None,
                }))
            }
            MmdbKind::Isp => {
                let result = self.inner.lookup(ip)?;
                if !result.has_data() {
                    return Ok(None);
                }
                let Some(rec) = result.decode::<geoip2::Isp>()? else {
                    return Ok(None);
                };
                Ok(Some(AsnRecord {
                    asn: rec.autonomous_system_number,
                    organization: rec
                        .autonomous_system_organization
                        .or(rec.organization)
                        .map(str::to_string),
                    isp: rec.isp.map(str::to_string),
                }))
            }
            _ => Ok(None),
        }
    }
}

fn world_from_city(rec: &geoip2::City<'_>) -> WorldRecord {
    let iso = rec
        .country
        .iso_code
        .or(rec.registered_country.iso_code)
        .map(str::to_ascii_uppercase);
    let (country_zh, country_en) = names_pair(&rec.country.names);
    let country_zh =
        country_zh.or_else(|| iso.as_deref().and_then(country_name_zh).map(str::to_string));
    let continent_code = rec.continent.code.map(str::to_ascii_uppercase);
    let (continent_zh, _) = names_pair(&rec.continent.names);
    let continent_zh = continent_zh.or_else(|| {
        continent_code
            .as_deref()
            .and_then(continent_name_zh)
            .map(str::to_string)
    });

    let (city_zh, city_en) = names_pair(&rec.city.names);
    let city = city_zh.or(city_en);

    let province = rec
        .subdivisions
        .first()
        .and_then(|s| names_pair(&s.names).0.or_else(|| names_pair(&s.names).1));
    let district = if rec.subdivisions.len() > 1 {
        rec.subdivisions
            .last()
            .and_then(|s| names_pair(&s.names).0.or_else(|| names_pair(&s.names).1))
            .filter(|d| Some(d) != city.as_ref())
    } else {
        None
    };

    WorldRecord {
        country_code: iso,
        country_zh,
        country_en,
        continent_code,
        continent_zh,
        province,
        city,
        district,
        latitude: rec.location.latitude,
        longitude: rec.location.longitude,
        timezone: rec.location.time_zone.map(str::to_string),
    }
}

fn world_from_country(rec: &geoip2::Country<'_>) -> WorldRecord {
    let iso = rec
        .country
        .iso_code
        .or(rec.registered_country.iso_code)
        .map(str::to_ascii_uppercase);
    let (country_zh, country_en) = names_pair(&rec.country.names);
    let country_zh =
        country_zh.or_else(|| iso.as_deref().and_then(country_name_zh).map(str::to_string));
    let continent_code = rec.continent.code.map(str::to_ascii_uppercase);
    let (continent_zh, _) = names_pair(&rec.continent.names);
    let continent_zh = continent_zh.or_else(|| {
        continent_code
            .as_deref()
            .and_then(continent_name_zh)
            .map(str::to_string)
    });
    WorldRecord {
        country_code: iso,
        country_zh,
        country_en,
        continent_code,
        continent_zh,
        ..Default::default()
    }
}

fn names_pair(names: &geoip2::Names<'_>) -> (Option<String>, Option<String>) {
    (
        names.simplified_chinese.map(str::to_string),
        names.english.map(str::to_string),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detects_database_kind() {
        assert_eq!(
            MmdbKind::from_database_type("GeoLite2-City"),
            MmdbKind::City
        );
        assert_eq!(
            MmdbKind::from_database_type("GeoLite2-Country"),
            MmdbKind::Country
        );
        assert_eq!(MmdbKind::from_database_type("GeoLite2-ASN"), MmdbKind::Asn);
        assert_eq!(MmdbKind::from_database_type("GeoIP2-ISP"), MmdbKind::Isp);
        assert_eq!(
            MmdbKind::from_database_type("GeoIP2-Enterprise"),
            MmdbKind::Enterprise
        );
        assert_eq!(
            MmdbKind::from_database_type("DBIP-City-Lite"),
            MmdbKind::City
        );
    }

    #[test]
    fn names_prefer_simplified_chinese() {
        let names = geoip2::Names {
            simplified_chinese: Some("美国"),
            english: Some("United States"),
            ..Default::default()
        };
        assert_eq!(
            names_pair(&names),
            (Some("美国".to_string()), Some("United States".to_string()))
        );
    }
}
