# data/ — IP 数据文件

本目录随仓库提供基础 IP 数据，**不会打包进 crate 或二进制**（见 `Cargo.toml` 的 `exclude`）。
程序启动时 `IpLocator::load_or_download` 会优先读取本目录；缺失时再从 Pek.RPanlServer
组件源按需下载。

## 仓库自带文件

| 文件 | 用途 | 许可 |
|------|------|------|
| `ip2region_v4.xdb` | 国内 IPv4 城市/省/运营商 | Apache-2.0（[ip2region](https://github.com/lionsoul2014/ip2region)） |
| `ip2region_v6.xdb` | 国内 IPv6 城市/省/运营商 | Apache-2.0（同上） |
| `dbip-country-lite-2026-10.mmdb` | 国外国家/洲（IPv4+IPv6） | CC BY 4.0（[DB-IP](https://db-ip.com)） |
| `dbip-asn-lite-2026-10.mmdb` | 国外 ASN/组织（IPv4+IPv6） | CC BY 4.0（DB-IP） |

> DB-IP 署名：IP Geolocation by DB-IP (https://db-ip.com)，CC BY 4.0。

## 需要城市级国外数据？

国外城市级（省/市/经纬度/时区）需要 City MMDB。推荐：

- **DB-IP City Lite**（免账号，CC BY 4.0）：运行 `scripts/fetch-data.ps1 -City`
  - 解压后约 121 MiB，超过 GitHub 单文件 100 MiB 限制，故默认不提交；
- **MaxMind GeoLite2-City**（需注册账号，GeoLite2 EULA）：下载后放到本目录。

识别规则（不区分大小写）：

- 城市/国家：`*.mmdb` 且文件名含 `city` 或 `country`（优先 `city`）；
- ASN/ISP：`*.mmdb` 且文件名含 `asn` 或 `isp`；
- IPv4 xdb：`ip2region_v4.xdb`（兼容 `ip2region.xdb`）；
- IPv6 xdb：`ip2region_v6.xdb`。

## 更新数据

1. 运行 `scripts/fetch-data.ps1` 更新 ip2region / DB-IP Country+ASN；
2. 运行 `scripts/fetch-data.ps1 -City` 更新 DB-IP City；
3. 或从数据商下载后覆盖本目录文件；
4. 打包上传到 Pek.RPanlServer：

   ```powershell
   .\scripts\pack-ipdb.ps1 -File .\data\ip2region_v4.xdb -Version 2026.10
   ```

   > 版本号建议用数据日期（如 `2026.10`），客户端 `ensure_updated` 会按数字段比较版本。
