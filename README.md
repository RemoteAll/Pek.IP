# Pek.IP

`Pek.IP` 是面向 Rust 服务端的离线 IP 定位库（crate `pek-ip`）：

- **国内**：读取 [ip2region](https://github.com/lionsoul2014/ip2region) xdb v3（IPv4 / IPv6，省/市/运营商，中文数据）；
- **国外**：读取 MaxMind GeoLite2 / GeoIP2、DB-IP 等 MMDB（国家/省/市/经纬度/时区/ASN）；
- **双源合并**：国内 IP 用 ip2region 的省市，经纬度/时区/区县由 MMDB 补齐；国外 IP 以 MMDB 为主；
- **特殊地址**：内网、回环、链路本地、组播、文档/保留地址直接识别，不查库；
- **按需下载**：从 Pek.RPanlServer 组件源下载数据包（Ed25519 验签 + SHA-256），本地缓存、离线可用。

数据文件随仓库 `data/` 提供（基础集：ip2region v4/v6 + DB-IP Country/ASN），但 **不会打包进 crate 或二进制**（`Cargo.toml` 的 `exclude`）。城市级 MMDB 体积较大，按需下载或自行放入 `data/`。

- 上传教程：[`docs/RPanel上传教程.md`](docs/RPanel上传教程.md)
- 下载授权/防刷流量协议：[`docs/下载授权协议.md`](docs/下载授权协议.md)

## 特性

| 能力 | 说明 |
|------|------|
| IPv4 / IPv6 | xdb v3 与 MMDB 均支持 |
| 国内城市级 | ip2region：省/市/运营商（中文） |
| 国外城市级 | GeoLite2-City / DB-IP City：国家/省/市/经纬度/时区 |
| ASN | GeoLite2-ASN / GeoIP2-ISP |
| 合并策略 | 国内省市以 xdb 为准，坐标/时区由 MMDB 补齐 |
| 线程安全 | 数据加载后只读，`Send + Sync`，查询无锁 |
| 按需下载 | 组件源目录 + Ed25519 签名 + SHA-256 + zip 解压 |
| 轻量 | xdb 解析纯标准库；MMDB 用纯 Rust `maxminddb`；无 C 依赖 |

## 数据源与许可

> `data/` 中的文件随仓库提供，但不会被 `cargo package`/二进制包含。下列数据请遵守各自许可：

| 数据 | 用途 | 仓库自带 | 许可/获取方式 |
|------|------|----------|---------------|
| `ip2region_v4.xdb` / `ip2region_v6.xdb` | 国内 IPv4 / IPv6 城市级 | ✅ | Apache-2.0，[ip2region 仓库](https://github.com/lionsoul2014/ip2region) `data/` |
| `dbip-country-lite-*.mmdb` | 国外国家/洲 | ✅ | CC BY 4.0，[db-ip.com](https://db-ip.com) |
| `dbip-asn-lite-*.mmdb` | 国外 ASN/组织 | ✅ | CC BY 4.0，同上 |
| `dbip-city-lite-*.mmdb` | 国外城市级 | 可选（约 121 MiB） | CC BY 4.0，`scripts/fetch-data.ps1 -City` |
| `GeoLite2-City.mmdb` / `GeoLite2-ASN.mmdb` | 国外城市/ASN | 自行放入 | MaxMind GeoLite2 EULA，需注册账号 |

**xdb 版本要求**：仅支持 **xdb v3**（ip2region 2025-09 之后的数据；`version=3`、`runtime_ptr_bytes=4`）。
旧版 v2 xdb 会返回明确错误，请重新下载 v4/v6 数据。

## 快速开始

```toml
[dependencies]
pek-ip = { path = "../Pek.IP" }
```

### 本地数据

```rust
use pek_ip::IpLocator;

fn main() -> Result<(), pek_ip::Error> {
    let locator = IpLocator::builder()
        .xdb_v4("data/ip2region_v4.xdb")?
        .xdb_v6("data/ip2region_v6.xdb")?
        // 仓库自带的 DB-IP Country/ASN；有 City 库时换成 dbip-city-lite-*.mmdb
        .world("data/dbip-country-lite-2026-10.mmdb")?
        .world_asn("data/dbip-asn-lite-2026-10.mmdb")?
        .build();

    let loc = locator.lookup_str("114.114.114.114")?;
    println!("{}（{}）", loc.display_short(), loc.source.label());
    // 中国 江苏省 南京市（ip2region + ASN）
    Ok(())
}
```

### 启动时自动加载/下载（推荐：零业务配置）

```rust
# fn main() -> Result<(), pek_ip::Error> {
let locator = pek_ip::IpLocator::load_default()?;
# Ok(()) }
```

库自动读取 `Config/Ip.toml`（不存在会按模板生成），平台地址默认固定为 `https://p.sc8.fun`；
需要更换 RPanelServer 时只改这一项：

```toml
# Config/Ip.toml
StoreUrl = "https://p.sc8.fun"
PubKey = ""              # 平台 Ed25519 公钥；非空 = 强制验签
DataDir = "data"
CacheDir = "Data/IpCache"
StateDir = "Data/IpClient"
AutoEnroll = true
DownloadV6 = false
```

行为：本地 `data/` → 本地缓存 → 组件源下载（Ed25519 验签 + SHA-256）→ 本地缺失/网络失败按 `allow_partial` 跳过或报错；**授权被拒、签名/格式错误不会被跳过**。

**自动授权（防 PCDN / 防批量转售，业务系统零身份配置）**：`AutoEnroll = true` 时，首次下载会自动：

1. 在 `StateDir` 生成 `install.id` 与 Ed25519 私钥；
2. 向平台 `POST /api/ipdb/enroll` 注册（平台按来源 CIDR 自动批准，或管理员在「IP 授权」页批准）；
3. 获取平台签名的 License；
4. 每次下载前用私钥签一张 5 分钟票据，向 `POST /api/ipdb/ticket` 换取一次性下载 URL（平台验签、查每日配额、防重放）。

平台不支持授权接口（旧版本）时自动回退公开下载；授权接口明确返回 403/401 时**不会**回退。业务系统无需配置 clientId/密钥，只需 `StoreUrl` + `PubKey`。详见 [`docs/下载授权协议.md`](docs/下载授权协议.md)。

### 高级：代码内指定

```rust
use pek_ip::{IpDbManagerConfig, IpLocator, LocatorLoadOptions};

# fn main() -> Result<(), pek_ip::Error> {
let locator = IpLocator::load_or_download(
    IpDbManagerConfig {
        store_url: "https://p.sc8.fun".into(),
        pubkey: "<平台 Ed25519 公钥 hex>".into(),
        cache_dir: Some("Data/IpCache".into()),
        ..Default::default()
    },
    LocatorLoadOptions {
        data_dir: "data".into(),
        load_v6: true,
        download_v6: false,
        ..Default::default()
    },
)?;
# Ok(()) }
```

### 按需下载（Pek.RPanlServer 组件源）

```toml
[dependencies]
pek-ip = { path = "../Pek.IP", features = ["manager"] }
```

```rust
use pek_ip::{IpDbManager, IpDbManagerConfig, DATASET_IP2REGION_V4};

fn main() -> Result<(), pek_ip::Error> {
    let mgr = IpDbManager::new(IpDbManagerConfig {
        store_url: "https://p.sc8.fun".into(),
        pubkey: "<平台 Ed25519 公钥 hex>".into(),
        ..Default::default()
    })?;

    let db = mgr.ensure_updated(DATASET_IP2REGION_V4)?;
    let locator = pek_ip::IpLocator::builder()
        .xdb_v4(db.file_with_ext("xdb").unwrap())?
        .build();
    println!("{}", locator.lookup_str("223.5.5.5")?.display_short());
    Ok(())
}
```

- `ensure`：本地有任意版本直接返回（离线优先）；没有才联网。
- `ensure_updated`：联网检查最新版本，下载/安装；本地同版本且 SHA-256 一致则跳过。
- `store_url` 为空 = **仅本地缓存模式**（离线部署/测试）。
- `auth`：默认关闭；`Config/Ip.toml` 的 `AutoEnroll=true` 会启用自动注册/票据。需要在代码内直接使用时，可设置 `IpDbManagerConfig { auth: IpAuthConfig { enabled: true, platform_pubkey: Some(pubkey), ..Default::default() }, .. }`。
- `access_token`：仍保留为 `Authorization: Bearer` 透传（平台后续可叠加统一网关令牌）。

## 结果结构

```rust
pub struct IpLocation {
    pub ip: IpAddr,
    pub country: Option<String>,        // 中文优先
    pub country_code: Option<String>,   // ISO alpha-2
    pub country_en: Option<String>,
    pub continent: Option<String>,
    pub continent_code: Option<String>,
    pub province: Option<String>,
    pub city: Option<String>,
    pub district: Option<String>,
    pub isp: Option<String>,
    pub asn: Option<u32>,
    pub as_org: Option<String>,
    pub latitude: Option<f64>,
    pub longitude: Option<f64>,
    pub timezone: Option<String>,
    pub is_china: bool,                 // CN / “中国”
    pub is_private: bool,               // 非公网地址
    pub ip_class: Option<IpClass>,      // 内网/回环/组播…
    pub source: DataSource,             // Ip2Region/Mmdb/Merged/Reserved/Unknown
}
```

### 合并规则

1. 内网/回环/保留地址直接返回，`source = Reserved`；
2. 国内 IP（ip2region 判定为中国）：
   - 国家/省/市/运营商用 ip2region；
   - 经纬度/时区/区县用 MMDB 补齐；
   - `source = Merged`（有 MMDB）或 `Ip2Region`；
3. 国外 IP：
   - 以 MMDB 国家/省/市/坐标为主；
   - MMDB 缺失字段由 ip2region 补齐；实际使用 ip2region 时 `source = Merged`；
4. ASN/ISP 最后合并；`isp` 缺失时回退 ASN 组织名；
5. 国家中文名：优先 MMDB `zh-CN`，否则按 ISO 代码查内置 CLDR 表。

### 特殊地址

`IpClass` 覆盖：

- IPv4：`0.0.0.0/8`、`10/8`、`100.64/10`、`127/8`、`169.254/16`、`172.16/12`、`192.168/16`、`192.0.0/24`、`192.0.2/24`、`198.18/15`、`198.51.100/24`、`203.0.113/24`、`224/4`、`240/4`；
- IPv6：`::`、`::1`、`64:ff9b::/96`、`100::/64`、`2001:db8::/32`、`2001:10::/28`、`fc00::/7`、`fe80::/10`、`ff00::/8`；
- IPv4-mapped IPv6（`::ffff:a.b.c.d`）自动归一为 IPv4。

## 上传到 Pek.RPanlServer

完整图文步骤见 [`docs/RPanel上传教程.md`](docs/RPanel上传教程.md)。平台组件源地址：`{平台}/components/catalog.json`，签名同址 `.sig`。简要步骤：

1. 用脚本打包数据：

   ```powershell
   .\scripts\pack-ipdb.ps1 -File .\data\ip2region_v4.xdb
   # 或显式指定：
   .\scripts\pack-ipdb.ps1 -File .\data\GeoLite2-City.mmdb -Id geolite2-city -Version 2026.10
   ```

2. 打开 Pek.RPanlServer 管理端 →「下载管理 / 组件」→ 上传组件：
   - 组件 id：脚本输出（如 `ip2region-v4`）
   - 版本：脚本输出（如 `2026.10.21`）
   - 目标平台：`any`
   - 分类：`ip-database`
   - 包：`dist/{id}-{version}-any.zip`
3. 确认组件已「上架」；客户端即可 `ensure_updated("ip2region-v4")`。

### 数据集 id 约定

| 常量 | id | 内容 |
|------|----|------|
| `DATASET_IP2REGION_V4` | `ip2region-v4` | ip2region IPv4 xdb |
| `DATASET_IP2REGION_V6` | `ip2region-v6` | ip2region IPv6 xdb |
| `DATASET_GEOLITE2_CITY` | `geolite2-city` | GeoLite2/DB-IP City MMDB |
| `DATASET_GEOLITE2_ASN` | `geolite2-asn` | GeoLite2/GeoIP2 ASN/ISP MMDB |
| `DATASET_DBIP_CITY` | `dbip-city` | DB-IP City MMDB |

zip 内可直接放数据文件（可含子目录），管理器会递归查找 `.xdb` / `.mmdb`。

### 包体积

- ip2region v4 xdb ≈ 11 MiB，zip 后 ≈ 4 MiB；v6 xdb ≈ 37 MiB，zip 后通常 < 20 MiB；
- GeoLite2/DB-IP City 原始约 60–127 MiB，zip 后可能较大；
- Pek.RPanlServer 组件包上限已放宽到 **128 MiB**（`components.rs` 的 `MAX_PACKAGE_BYTES`）；
- 超过时请继续调大上限，或只上传精简城市库。

## 打包脚本

`scripts/pack-ipdb.ps1`：

- 自动识别 `ip2region_v4.xdb` / `ip2region_v6.xdb` / `GeoLite2-City.mmdb` / `GeoLite2-ASN.mmdb` / `dbip-city*.mmdb`；
- 生成 `dataset.json`（格式/数据源/许可说明）；
- 使用 .NET `ZipFile` 以 `Optimal` 压缩，输出 `{id}-{version}-any.zip`；
- 计算 SHA-256 并打印平台上传参数；
- 临时文件先写 `.new`，成功后再替换，避免留下半包。

## 构建与测试

```powershell
cargo check
cargo test
cargo test --all-features   # 含 manager（会编译 dhrust 的 HTTP/TLS/插件/zip）
cargo clippy --all-targets --all-features
```

真实数据验证：

```powershell
$env:IP2REGION_V4 = 'data\ip2region_v4.xdb'
$env:GEOLITE2_CITY = 'data\GeoLite2-City.mmdb'
$env:GEOLITE2_ASN = 'data\GeoLite2-ASN.mmdb'
cargo run --example lookup -- 114.114.114.114 8.8.8.8 240e:3b7:3273:51d0::1 192.168.1.1
```

## 性能

- xdb 全量读入内存（v4 ≈ 11 MiB、v6 ≈ 37 MiB），查询为纯内存二分/向量索引，无锁；
- MMDB 使用 `maxminddb` 解码，数据同样常驻内存；
- 典型单次查询为微秒级；高 QPS 场景建议把 `IpLocator` 放在 `Arc` 中复用。

## 说明

- 本库只做“离线解析 + 按需下载”；仓库自带数据均为可再分发许可（Apache-2.0 / CC BY 4.0），商业数据请自行确认许可；
- “按需支付/授权下载”目前通过 `access_token` 预留客户端能力，协议见 `docs/下载授权协议.md`，平台侧订单/授权/计费待实现；
- 地理位置为 IP 归属地估算，不保证精确到街道/门牌，请勿用于人身定位等场景。

## 许可

代码 MIT。数据文件遵循各自许可（见上表）。
