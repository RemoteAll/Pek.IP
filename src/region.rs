//! ip2region 地域字符串解析。
//!
//! xdb v3 的标准格式为：`国家|省份|城市|ISP|ISO-3166-1-alpha2`，
//! 例如：
//!
//! - `中国|福建省|福州市|中国电信|CN`
//! - `Australia|Queensland|0|0|AU`
//! - `United States|Texas|0|Oracle Svenska AB|US`
//!
//! 同时兼容旧版 v2 格式：`国家|区域|省份|城市|ISP`，例如 `中国|0|广东省|深圳市|电信`。

use crate::country::{country_code_from_name, country_name_zh};

/// 解析后的国内/通用地域记录。
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct CnRegion {
    /// 国家/地区名称（数据原样）
    pub country: Option<String>,
    /// ISO alpha-2（可能来自数据，或按中文名反查）
    pub country_code: Option<String>,
    /// 省/州
    pub province: Option<String>,
    /// 城市
    pub city: Option<String>,
    /// 运营商
    pub isp: Option<String>,
    /// 是否中国（含“中国”开头 / `CN`）
    pub is_china: bool,
    /// 数据明确标记为保留/内网（如 `Reserved`、`内网IP`）
    pub reserved: bool,
}

impl CnRegion {
    /// 解析 xdb 地域字符串（v3，兼容 v2）。
    pub fn parse(region: &str) -> Self {
        let fields: Vec<&str> = region.split('|').map(str::trim).collect();
        let reserved = fields.iter().any(|f| {
            f.eq_ignore_ascii_case("reserved")
                || *f == "内网IP"
                || *f == "局域网"
                || *f == "保留地址"
        });
        if reserved {
            return Self {
                reserved: true,
                ..Default::default()
            };
        }

        let (country, province, city, isp, code) = match fields.as_slice() {
            [] => ("", "", "", "", None),
            [country] => (*country, "0", "0", "0", None),
            [country, province] => (*country, *province, "0", "0", None),
            [country, province, city] => (*country, *province, *city, "0", None),
            [country, _region, province, city] => (*country, *province, *city, "0", None),
            [country, f1, f2, f3, f4, ..] => {
                let iso_like = is_iso_like(f4);
                if iso_like {
                    // v3: country|province|city|isp|iso
                    (*country, *f1, *f2, *f3, Some(*f4))
                } else if *f1 == "0" && *f2 != "0" {
                    // v2: country|region|province|city|isp
                    (*country, *f2, *f3, *f4, None)
                } else {
                    // v3（ISO 缺失）
                    (*country, *f1, *f2, *f3, None)
                }
            }
        };

        let country = clean(country);
        let province = clean(province);
        let city = clean(city);
        let isp = clean(isp);
        let code = code
            .and_then(clean)
            .map(|c| c.to_ascii_uppercase())
            .filter(|c| c.len() == 2 && c.chars().all(|ch| ch.is_ascii_alphabetic()))
            .or_else(|| {
                country
                    .as_deref()
                    .and_then(country_code_from_name)
                    .map(str::to_string)
            });

        let looks_china = country.as_deref().is_some_and(|c| c.starts_with("中国"))
            || code.as_deref() == Some("CN")
            || (country.is_none() && province.as_deref().is_some_and(is_chinese_province));
        let code = if code.is_none() && looks_china {
            Some("CN".to_string())
        } else {
            code
        };

        let country = match (country, looks_china) {
            (None, true) => Some("中国".to_string()),
            (Some(c), true) if c == "China" => Some("中国".to_string()),
            (c, _) => c,
        };

        Self {
            country,
            country_code: code,
            province,
            city,
            isp,
            is_china: looks_china,
            reserved: false,
        }
    }

    /// 是否没有任何有效字段。
    pub fn is_empty(&self) -> bool {
        self.country.is_none()
            && self.province.is_none()
            && self.city.is_none()
            && self.isp.is_none()
    }

    /// 展示用名称：优先中文名；有 ISO 时按 ISO 翻译。
    pub fn country_display_zh(&self) -> Option<String> {
        if let Some(code) = &self.country_code
            && let Some(name) = country_name_zh(code)
        {
            return Some(name.to_string());
        }
        self.country.clone()
    }
}

fn clean(value: &str) -> Option<String> {
    let text = value.trim();
    if text.is_empty()
        || text == "0"
        || text.eq_ignore_ascii_case("reserved")
        || text == "内网IP"
        || text == "局域网"
        || text == "未知"
    {
        None
    } else {
        Some(text.to_string())
    }
}

fn is_iso_like(value: &str) -> bool {
    let text = value.trim();
    text.len() == 2 && text.chars().all(|c| c.is_ascii_alphabetic()) && text != "0"
}

fn is_chinese_province(value: &str) -> bool {
    value.ends_with('省')
        || value.ends_with('市')
        || value.ends_with("自治区")
        || value.ends_with("特别行政区")
        || value == "台湾"
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_v3_china() {
        let r = CnRegion::parse("中国|福建省|福州市|中国电信|CN");
        assert_eq!(r.country.as_deref(), Some("中国"));
        assert_eq!(r.country_code.as_deref(), Some("CN"));
        assert_eq!(r.province.as_deref(), Some("福建省"));
        assert_eq!(r.city.as_deref(), Some("福州市"));
        assert_eq!(r.isp.as_deref(), Some("中国电信"));
        assert!(r.is_china);
        assert!(!r.reserved);
        assert_eq!(r.country_display_zh().as_deref(), Some("中国"));
    }

    #[test]
    fn parses_v3_overseas() {
        let r = CnRegion::parse("Australia|Queensland|0|0|AU");
        assert_eq!(r.country.as_deref(), Some("Australia"));
        assert_eq!(r.country_code.as_deref(), Some("AU"));
        assert_eq!(r.province.as_deref(), Some("Queensland"));
        assert_eq!(r.city, None);
        assert_eq!(r.isp, None);
        assert!(!r.is_china);
        assert_eq!(r.country_display_zh().as_deref(), Some("澳大利亚"));
    }

    #[test]
    fn parses_v2_china() {
        let r = CnRegion::parse("中国|0|广东省|深圳市|电信");
        assert_eq!(r.country.as_deref(), Some("中国"));
        assert_eq!(r.country_code.as_deref(), Some("CN"));
        assert_eq!(r.province.as_deref(), Some("广东省"));
        assert_eq!(r.city.as_deref(), Some("深圳市"));
        assert_eq!(r.isp.as_deref(), Some("电信"));
        assert!(r.is_china);
    }

    #[test]
    fn parses_v2_without_country() {
        let r = CnRegion::parse("0|0|浙江省|杭州市|0");
        assert_eq!(r.country.as_deref(), Some("中国"));
        assert_eq!(r.country_code.as_deref(), Some("CN"));
        assert_eq!(r.province.as_deref(), Some("浙江省"));
        assert_eq!(r.city.as_deref(), Some("杭州市"));
        assert!(r.is_china);
    }

    #[test]
    fn reserved_markers() {
        for text in [
            "0|0|Reserved|Reserved|Reserved",
            "0|0|内网IP|内网IP",
            "Reserved|0|0|0|0",
        ] {
            let r = CnRegion::parse(text);
            assert!(r.reserved, "{text}");
            assert!(r.is_empty(), "{text}");
        }
    }

    #[test]
    fn empty_or_unknown() {
        assert!(CnRegion::parse("").is_empty());
        assert!(CnRegion::parse("0|0|0|0|0").is_empty());
        assert!(!CnRegion::parse("United States|Texas|0|Oracle Svenska AB|US").is_china);
    }
}
