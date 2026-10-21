//! ip2region xdb v3 读取器（IPv4 / IPv6，纯标准库实现）。
//!
//! ## 文件布局（xdb v3，2025-09 起）
//!
//! | 段 | 大小 | 说明 |
//! |----|------|------|
//! | Header | 256 B | 版本/索引策略/索引起止/ IP 版本/指针宽度 |
//! | Vector Index | 512 KiB | 256 × 256 个 `(start_ptr, end_ptr)`，按 IP 前两字节分桶 |
//! | Region Data | 动态 | 去重后的地域字符串（UTF-8） |
//! | Segment Index | 动态 | 二分索引记录（IPv4 14 B / IPv6 38 B） |
//!
//! Header 字段（小端）：`version u16`、`policy u16`、`create_time u32`、
//! `start_index_ptr u32`、`end_index_ptr u32`、`ip_version u16`、`runtime_ptr_bytes u16`。
//!
//! - `policy = 1`（VectorIndex）：先用向量索引缩小区间，再二分；
//! - `policy = 2`（BTreeIndex）：直接在全量二分索引中查找。
//!
//! 本实现将整个文件读入内存（v4 约 11 MiB、v6 约 37 MiB），查询无锁、线程安全，
//! 适合服务端高频查询；不依赖 mmap 或 C 库，可全平台交叉编译。

use std::net::{IpAddr, Ipv4Addr, Ipv6Addr};
use std::path::{Path, PathBuf};
use std::sync::Arc;

use crate::{Error, Result};

/// Header 长度。
const HEADER_LEN: usize = 256;
/// 向量索引列数（IP 第 1 字节）。
const VECTOR_COLS: usize = 256;
/// 向量索引行数（IP 第 2 字节）。
const VECTOR_ROWS: usize = 256;
/// 单个向量索引项长度（start u32 + end u32）。
const VECTOR_ENTRY_LEN: usize = 8;
/// 向量索引总长度（512 KiB）。
const VECTOR_LEN: usize = VECTOR_COLS * VECTOR_ROWS * VECTOR_ENTRY_LEN;
/// 地域数据段起始偏移。
const REGION_START: usize = HEADER_LEN + VECTOR_LEN;
/// IPv4 二分索引记录长度：start 4 + end 4 + len 2 + ptr 4。
const RECORD_LEN_V4: usize = 14;
/// IPv6 二分索引记录长度：start 16 + end 16 + len 2 + ptr 4。
const RECORD_LEN_V6: usize = 38;
/// 当前支持的 xdb 版本。
const XDB_VERSION: u16 = 3;
/// VectorIndex 策略。
const POLICY_VECTOR: u16 = 1;
/// BTreeIndex 策略。
const POLICY_BTREE: u16 = 2;
/// 索引指针宽度（字节）。
const PTR_BYTES: u16 = 4;

/// xdb 的 IP 版本。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum XdbIpVersion {
    /// IPv4
    V4,
    /// IPv6
    V6,
}

impl XdbIpVersion {
    /// IP 字节长度。
    pub fn ip_bytes(self) -> usize {
        match self {
            Self::V4 => 4,
            Self::V6 => 16,
        }
    }

    /// 二分索引记录长度。
    pub fn record_len(self) -> usize {
        match self {
            Self::V4 => RECORD_LEN_V4,
            Self::V6 => RECORD_LEN_V6,
        }
    }

    /// 人类可读名称。
    pub fn as_str(self) -> &'static str {
        match self {
            Self::V4 => "IPv4",
            Self::V6 => "IPv6",
        }
    }
}

/// xdb 索引策略。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum XdbIndexPolicy {
    /// 向量索引 + 二分。
    Vector,
    /// 全量二分。
    BTree,
}

impl XdbIndexPolicy {
    /// 人类可读名称。
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Vector => "VectorIndex",
            Self::BTree => "BTreeIndex",
        }
    }
}

#[derive(Clone, Debug)]
struct XdbHeader {
    version: u16,
    policy: XdbIndexPolicy,
    create_time: u32,
    start_index: usize,
    end_index: usize,
    ip_version: XdbIpVersion,
}

/// ip2region xdb v3 读取器。
///
/// 构造后不可变，可被多线程共享（`Send + Sync`）。
#[derive(Clone)]
pub struct XdbReader {
    bytes: Arc<[u8]>,
    header: XdbHeader,
    path: Option<PathBuf>,
}

impl std::fmt::Debug for XdbReader {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("XdbReader")
            .field("path", &self.path)
            .field("version", &self.header.version)
            .field("policy", &self.header.policy)
            .field("ip_version", &self.header.ip_version)
            .field("len", &self.bytes.len())
            .finish()
    }
}

impl XdbReader {
    /// 打开 xdb 文件（全量读入内存）。
    pub fn open(path: impl AsRef<Path>) -> Result<Self> {
        let path = path.as_ref();
        if !path.is_file() {
            return Err(Error::MissingFile(path.to_path_buf()));
        }
        let data = std::fs::read(path)?;
        let mut reader = Self::from_bytes(data)?;
        reader.path = Some(path.to_path_buf());
        Ok(reader)
    }

    /// 从字节构造（便于测试或从内存加载）。
    pub fn from_bytes(data: Vec<u8>) -> Result<Self> {
        Self::from_shared_bytes(Arc::from(data.into_boxed_slice()))
    }

    /// 从共享字节构造。
    pub fn from_shared_bytes(data: Arc<[u8]>) -> Result<Self> {
        let header = parse_header(&data)?;
        Ok(Self {
            bytes: data,
            header,
            path: None,
        })
    }

    /// 文件路径（从字节构造时为 `None`）。
    pub fn path(&self) -> Option<&Path> {
        self.path.as_deref()
    }

    /// xdb 版本（当前固定为 3）。
    pub fn version(&self) -> u16 {
        self.header.version
    }

    /// 索引策略。
    pub fn policy(&self) -> XdbIndexPolicy {
        self.header.policy
    }

    /// IP 版本。
    pub fn ip_version(&self) -> XdbIpVersion {
        self.header.ip_version
    }

    /// 文件生成时间（Unix 秒）。
    pub fn create_time(&self) -> u32 {
        self.header.create_time
    }

    /// 文件长度（字节）。
    pub fn len(&self) -> usize {
        self.bytes.len()
    }

    /// 是否为空文件（正常 xdb 不会为空）。
    pub fn is_empty(&self) -> bool {
        self.bytes.is_empty()
    }

    /// 查询原始地域字符串（如 `中国|福建省|福州市|中国电信|CN`）。
    ///
    /// - 未命中返回 `Ok(None)`；
    /// - IP 版本与数据库不匹配（如用 v4 库查 IPv6）返回 `Ok(None)`；
    /// - 文件损坏返回 `Err`。
    pub fn lookup(&self, ip: IpAddr) -> Result<Option<String>> {
        let Some(key) = self.key_of(ip) else {
            return Ok(None);
        };

        let ip_len = self.header.ip_version.ip_bytes();
        let rec_len = self.header.ip_version.record_len();
        let start_index = self.header.start_index;
        let end_index = self.header.end_index;

        let (lo_ptr, hi_ptr) = match self.header.policy {
            XdbIndexPolicy::Vector => {
                let bucket = self.bucket_of(key);
                let off = HEADER_LEN + bucket * VECTOR_ENTRY_LEN;
                let lo = read_u32(&self.bytes, off)? as usize;
                let hi = read_u32(&self.bytes, off + 4)? as usize;
                if lo == 0 || hi == 0 {
                    // 生成器约定：全 0 表示该桶无数据
                    return Ok(None);
                }
                (lo, hi)
            }
            XdbIndexPolicy::BTree => (start_index, end_index),
        };

        if lo_ptr < start_index
            || hi_ptr < lo_ptr
            || hi_ptr > end_index
            || !(lo_ptr - start_index).is_multiple_of(rec_len)
            || !(hi_ptr - start_index).is_multiple_of(rec_len)
        {
            return Err(Error::Xdb(format!(
                "索引区间非法：start={start_index} end={end_index} lo={lo_ptr} hi={hi_ptr} record={rec_len}"
            )));
        }

        let mut lo = (lo_ptr - start_index) / rec_len;
        let mut hi = (hi_ptr - start_index) / rec_len;

        while lo <= hi {
            let mid = lo + (hi - lo) / 2;
            let offset = start_index + mid * rec_len;
            let rec_start = self.read_ip(offset);
            let rec_end = self.read_ip(offset + ip_len);

            if key < rec_start {
                if mid == 0 {
                    break;
                }
                hi = mid - 1;
            } else if key > rec_end {
                lo = mid + 1;
            } else {
                let len_off = offset + ip_len * 2;
                let data_len = read_u16(&self.bytes, len_off)? as usize;
                let data_ptr = read_u32(&self.bytes, len_off + 2)? as usize;
                let data_end = data_ptr
                    .checked_add(data_len)
                    .ok_or_else(|| Error::Xdb("地域数据指针溢出".to_string()))?;
                if data_ptr < REGION_START || data_end > self.bytes.len() {
                    return Err(Error::Xdb(format!(
                        "地域数据越界：ptr={data_ptr} len={data_len} file={}",
                        self.bytes.len()
                    )));
                }
                if data_len == 0 {
                    return Ok(None);
                }
                let text = std::str::from_utf8(&self.bytes[data_ptr..data_end])
                    .map_err(|e| Error::Xdb(format!("地域数据不是 UTF-8：{e}")))?;
                return Ok(if text.is_empty() {
                    None
                } else {
                    Some(text.to_string())
                });
            }
        }
        Ok(None)
    }

    /// 查询 IPv4（u32 数值）。
    pub fn lookup_ipv4(&self, ip: u32) -> Result<Option<String>> {
        self.lookup(IpAddr::V4(Ipv4Addr::from(ip)))
    }

    /// 查询 IPv6（u128 数值）。
    pub fn lookup_ipv6(&self, ip: u128) -> Result<Option<String>> {
        self.lookup(IpAddr::V6(Ipv6Addr::from(ip)))
    }

    /// 计算数据库内的数值键；IP 版本不匹配时返回 `None`。
    fn key_of(&self, ip: IpAddr) -> Option<u128> {
        match (self.header.ip_version, ip) {
            (XdbIpVersion::V4, IpAddr::V4(v4)) => Some(u32::from(v4) as u128),
            (XdbIpVersion::V4, IpAddr::V6(v6)) => {
                v6.to_ipv4_mapped().map(|v4| u32::from(v4) as u128)
            }
            (XdbIpVersion::V6, IpAddr::V6(v6)) => Some(u128::from(v6)),
            (XdbIpVersion::V6, IpAddr::V4(v4)) => {
                // IPv4-mapped IPv6：::ffff:a.b.c.d
                Some((0xffff_u128 << 32) | u32::from(v4) as u128)
            }
        }
    }

    /// 向量索引桶号（IP 前两字节）。
    fn bucket_of(&self, key: u128) -> usize {
        match self.header.ip_version {
            XdbIpVersion::V4 => ((key >> 16) & 0xffff) as usize,
            XdbIpVersion::V6 => ((key >> 112) & 0xffff) as usize,
        }
    }

    /// 读取一条索引记录中的 IP 键。
    fn read_ip(&self, offset: usize) -> u128 {
        match self.header.ip_version {
            XdbIpVersion::V4 => {
                u32::from_le_bytes(self.bytes[offset..offset + 4].try_into().unwrap()) as u128
            }
            XdbIpVersion::V6 => {
                u128::from_be_bytes(self.bytes[offset..offset + 16].try_into().unwrap())
            }
        }
    }
}

fn parse_header(data: &[u8]) -> Result<XdbHeader> {
    if data.len() < REGION_START {
        return Err(Error::Xdb(format!(
            "文件过小（{} 字节，至少需要 {} 字节）",
            data.len(),
            REGION_START
        )));
    }
    let version = read_u16(data, 0)?;
    if version != XDB_VERSION {
        return Err(Error::Unsupported(format!(
            "xdb 版本 {version} 不受支持（需要 v3；请使用 ip2region 2025-09 之后的数据文件）"
        )));
    }
    let policy_raw = read_u16(data, 2)?;
    let policy = match policy_raw {
        POLICY_VECTOR => XdbIndexPolicy::Vector,
        POLICY_BTREE => XdbIndexPolicy::BTree,
        other => {
            return Err(Error::Xdb(format!("未知索引策略：{other}")));
        }
    };
    let create_time = read_u32(data, 4)?;
    let start_index = read_u32(data, 8)? as usize;
    let end_index = read_u32(data, 12)? as usize;
    let ip_version = match read_u16(data, 16)? {
        4 => XdbIpVersion::V4,
        6 => XdbIpVersion::V6,
        other => {
            return Err(Error::Xdb(format!("未知 IP 版本：{other}")));
        }
    };
    let ptr_bytes = read_u16(data, 18)?;
    if ptr_bytes != PTR_BYTES {
        return Err(Error::Unsupported(format!(
            "索引指针宽度 {ptr_bytes} 字节不受支持（仅支持 4 字节）"
        )));
    }
    let rec_len = ip_version.record_len();
    if start_index < REGION_START || start_index > end_index {
        return Err(Error::Xdb(format!(
            "索引起止非法：start={start_index} end={end_index} region_start={REGION_START}"
        )));
    }
    if !(end_index - start_index).is_multiple_of(rec_len) {
        return Err(Error::Xdb(format!(
            "索引区间未按记录长度对齐：start={start_index} end={end_index} record={rec_len}"
        )));
    }
    if end_index
        .checked_add(rec_len)
        .is_none_or(|end| end > data.len())
    {
        return Err(Error::Xdb(format!(
            "索引越界：end={end_index} record={rec_len} file={}",
            data.len()
        )));
    }
    Ok(XdbHeader {
        version,
        policy,
        create_time,
        start_index,
        end_index,
        ip_version,
    })
}

fn read_u16(data: &[u8], offset: usize) -> Result<u16> {
    let end = offset
        .checked_add(2)
        .ok_or_else(|| Error::Xdb("读取越界".to_string()))?;
    data.get(offset..end)
        .map(|b| u16::from_le_bytes([b[0], b[1]]))
        .ok_or_else(|| Error::Xdb(format!("读取 u16 越界：offset={offset}")))
}

fn read_u32(data: &[u8], offset: usize) -> Result<u32> {
    let end = offset
        .checked_add(4)
        .ok_or_else(|| Error::Xdb("读取越界".to_string()))?;
    data.get(offset..end)
        .map(|b| u32::from_le_bytes([b[0], b[1], b[2], b[3]]))
        .ok_or_else(|| Error::Xdb(format!("读取 u32 越界：offset={offset}")))
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use std::collections::HashMap;

    /// 构造一个最小 xdb v3（IPv4）。
    pub(crate) fn build_xdb_v4(records: &[(u32, u32, &str)], policy: u16) -> Vec<u8> {
        let mut region_ptr: HashMap<&str, u32> = HashMap::new();
        let mut region_data = Vec::new();
        let mut cursor = REGION_START as u32;
        for (_, _, region) in records {
            if !region_ptr.contains_key(region) {
                region_ptr.insert(region, cursor);
                region_data.extend_from_slice(region.as_bytes());
                cursor += region.len() as u32;
            }
        }
        let start_index = REGION_START + region_data.len();
        let end_index = start_index + (records.len() - 1) * RECORD_LEN_V4;

        let mut vector = vec![0u8; VECTOR_LEN];
        let mut index = Vec::with_capacity(records.len() * RECORD_LEN_V4);
        for (start, end, region) in records {
            let ptr = start_index + index.len();
            index.extend_from_slice(&start.to_le_bytes());
            index.extend_from_slice(&end.to_le_bytes());
            let text = region.as_bytes();
            index.extend_from_slice(&(text.len() as u16).to_le_bytes());
            index.extend_from_slice(&region_ptr[*region].to_le_bytes());
            if policy == POLICY_VECTOR {
                let bucket = ((*start >> 16) & 0xffff) as usize;
                let off = bucket * VECTOR_ENTRY_LEN;
                if vector[off..off + 4] == [0, 0, 0, 0] {
                    vector[off..off + 4].copy_from_slice(&(ptr as u32).to_le_bytes());
                }
                vector[off + 4..off + 8].copy_from_slice(&(ptr as u32).to_le_bytes());
            }
        }

        let mut out = vec![0u8; start_index];
        out[0..2].copy_from_slice(&XDB_VERSION.to_le_bytes());
        out[2..4].copy_from_slice(&policy.to_le_bytes());
        out[8..12].copy_from_slice(&(start_index as u32).to_le_bytes());
        out[12..16].copy_from_slice(&(end_index as u32).to_le_bytes());
        out[16..18].copy_from_slice(&4u16.to_le_bytes());
        out[18..20].copy_from_slice(&PTR_BYTES.to_le_bytes());
        out[HEADER_LEN..REGION_START].copy_from_slice(&vector);
        out[REGION_START..start_index].copy_from_slice(&region_data);
        out.extend_from_slice(&index);
        out
    }

    /// 构造一个最小 xdb v3（IPv6）。
    pub(crate) fn build_xdb_v6(records: &[(u128, u128, &str)], policy: u16) -> Vec<u8> {
        let mut region_ptr: HashMap<&str, u32> = HashMap::new();
        let mut region_data = Vec::new();
        let mut cursor = REGION_START as u32;
        for (_, _, region) in records {
            if !region_ptr.contains_key(region) {
                region_ptr.insert(region, cursor);
                region_data.extend_from_slice(region.as_bytes());
                cursor += region.len() as u32;
            }
        }
        let start_index = REGION_START + region_data.len();
        let end_index = start_index + (records.len() - 1) * RECORD_LEN_V6;

        let mut vector = vec![0u8; VECTOR_LEN];
        let mut index = Vec::with_capacity(records.len() * RECORD_LEN_V6);
        for (start, end, region) in records {
            let ptr = start_index + index.len();
            index.extend_from_slice(&start.to_be_bytes());
            index.extend_from_slice(&end.to_be_bytes());
            let text = region.as_bytes();
            index.extend_from_slice(&(text.len() as u16).to_le_bytes());
            index.extend_from_slice(&region_ptr[*region].to_le_bytes());
            if policy == POLICY_VECTOR {
                let bucket = ((*start >> 112) & 0xffff) as usize;
                let off = bucket * VECTOR_ENTRY_LEN;
                if vector[off..off + 4] == [0, 0, 0, 0] {
                    vector[off..off + 4].copy_from_slice(&(ptr as u32).to_le_bytes());
                }
                vector[off + 4..off + 8].copy_from_slice(&(ptr as u32).to_le_bytes());
            }
        }

        let mut out = vec![0u8; start_index];
        out[0..2].copy_from_slice(&XDB_VERSION.to_le_bytes());
        out[2..4].copy_from_slice(&policy.to_le_bytes());
        out[8..12].copy_from_slice(&(start_index as u32).to_le_bytes());
        out[12..16].copy_from_slice(&(end_index as u32).to_le_bytes());
        out[16..18].copy_from_slice(&6u16.to_le_bytes());
        out[18..20].copy_from_slice(&PTR_BYTES.to_le_bytes());
        out[HEADER_LEN..REGION_START].copy_from_slice(&vector);
        out[REGION_START..start_index].copy_from_slice(&region_data);
        out.extend_from_slice(&index);
        out
    }

    #[test]
    fn lookup_v4_boundaries() {
        let data = build_xdb_v4(
            &[
                (0x0100_0000, 0x0100_00ff, "澳大利亚|昆士兰|0|0|AU"),
                (0x0100_0100, 0x0100_01ff, "中国|福建省|福州市|中国电信|CN"),
                (0x0100_0200, 0x0100_02ff, "美国|德克萨斯州|0|0|US"),
            ],
            POLICY_VECTOR,
        );
        let db = XdbReader::from_bytes(data).unwrap();
        assert_eq!(db.version(), 3);
        assert_eq!(db.policy(), XdbIndexPolicy::Vector);
        assert_eq!(db.ip_version(), XdbIpVersion::V4);
        assert_eq!(
            db.lookup("1.0.0.1".parse().unwrap()).unwrap().as_deref(),
            Some("澳大利亚|昆士兰|0|0|AU")
        );
        assert_eq!(
            db.lookup("1.0.1.0".parse().unwrap()).unwrap().as_deref(),
            Some("中国|福建省|福州市|中国电信|CN")
        );
        assert_eq!(
            db.lookup("1.0.1.255".parse().unwrap()).unwrap().as_deref(),
            Some("中国|福建省|福州市|中国电信|CN")
        );
        assert_eq!(
            db.lookup("1.0.2.128".parse().unwrap()).unwrap().as_deref(),
            Some("美国|德克萨斯州|0|0|US")
        );
        assert_eq!(db.lookup("1.0.3.0".parse().unwrap()).unwrap(), None);
        assert_eq!(db.lookup("8.8.8.8".parse().unwrap()).unwrap(), None);
    }

    #[test]
    fn lookup_v4_btree_policy() {
        let data = build_xdb_v4(
            &[
                (0x0100_0000, 0x0100_ffff, "A|B|C|D|AA"),
                (0x0200_0000, 0x0200_ffff, "E|F|G|H|BB"),
            ],
            POLICY_BTREE,
        );
        let db = XdbReader::from_bytes(data).unwrap();
        assert_eq!(db.policy(), XdbIndexPolicy::BTree);
        assert_eq!(
            db.lookup("2.0.0.1".parse().unwrap()).unwrap().as_deref(),
            Some("E|F|G|H|BB")
        );
        assert_eq!(db.lookup("3.0.0.1".parse().unwrap()).unwrap(), None);
    }

    #[test]
    fn lookup_v6() {
        let start: u128 = "240e:3b7:3273:51d0::".parse::<Ipv6Addr>().unwrap().into();
        let end: u128 = "240e:3b7:3273:51ff:ffff:ffff:ffff:ffff"
            .parse::<Ipv6Addr>()
            .unwrap()
            .into();
        let data = build_xdb_v6(&[(start, end, "中国|广东省|深圳市|电信|CN")], POLICY_VECTOR);
        let db = XdbReader::from_bytes(data).unwrap();
        assert_eq!(db.ip_version(), XdbIpVersion::V6);
        assert_eq!(
            db.lookup("240e:3b7:3273:51d0:cd38:8ae1:e3c0:b708".parse().unwrap())
                .unwrap()
                .as_deref(),
            Some("中国|广东省|深圳市|电信|CN")
        );
        assert_eq!(
            db.lookup("240e:3b7:3273:5200::".parse().unwrap()).unwrap(),
            None
        );
    }

    #[test]
    fn mismatch_version_returns_none() {
        let data = build_xdb_v4(&[(0x0100_0000, 0x0100_ffff, "A|B|C|D|AA")], POLICY_VECTOR);
        let db = XdbReader::from_bytes(data).unwrap();
        assert_eq!(db.lookup("2001:db8::1".parse().unwrap()).unwrap(), None);
    }

    #[test]
    fn rejects_truncated_or_bad_header() {
        let err = XdbReader::from_bytes(vec![0u8; 100]).unwrap_err();
        assert!(matches!(err, Error::Xdb(_)));

        let mut data = build_xdb_v4(&[(0x0100_0000, 0x0100_ffff, "A|B|C|D|AA")], POLICY_VECTOR);
        data[0..2].copy_from_slice(&2u16.to_le_bytes());
        let err = XdbReader::from_bytes(data).unwrap_err();
        assert!(matches!(err, Error::Unsupported(_)));
    }
}
