# 上传 IP 数据到 Pek.RPanlServer（教程）

> 目标：仓库 `data/` 只作为**源数据**，不进入 crate/二进制；把数据打包上传到
> Pek.RPanlServer，业务系统启动时本地有就用本地，没有就按需下载。

## 0. 准备数据

仓库自带：

```text
data/
├─ ip2region_v4.xdb
├─ ip2region_v6.xdb
├─ dbip-country-lite-2026-10.mmdb
└─ dbip-asn-lite-2026-10.mmdb
```

城市级国外数据（可选，推荐）：

```powershell
# DB-IP City Lite（免账号，CC BY 4.0；约 60MB 压缩 / 127MB 解压）
.\scripts\fetch-data.ps1 -City

# 或从 MaxMind 下载 GeoLite2-City.mmdb / GeoLite2-ASN.mmdb 放入 data/
```

## 1. 打包组件

```powershell
# 国内 IPv4
.\scripts\pack-ipdb.ps1 -File .\data\ip2region_v4.xdb -Version 2026.10

# 国内 IPv6（可选）
.\scripts\pack-ipdb.ps1 -File .\data\ip2region_v6.xdb -Version 2026.10

# 国外城市（DB-IP City Lite）
.\scripts\pack-ipdb.ps1 -File .\data\dbip-city-lite-2026-10.mmdb -Id dbip-city -Version 2026.10

# 国外 ASN
.\scripts\pack-ipdb.ps1 -File .\data\dbip-asn-lite-2026-10.mmdb -Id geolite2-asn -Version 2026.10
```

产物：

```text
dist/
├─ ip2region-v4-2026.10-any.zip
├─ ip2region-v6-2026.10-any.zip
├─ dbip-city-2026.10-any.zip
└─ geolite2-asn-2026.10-any.zip
```

每个包内包含数据文件与 `dataset.json`（格式/来源/许可说明）。

## 2. 启动 Pek.RPanlServer 并登录

```powershell
# 开发机示例（默认端口 5502；生产按实际部署）
cd G:\Code\Rust\Pek.RPanlServer
cargo run --release
```

浏览器打开 `http://127.0.0.1:5502/`，用管理员账号登录。

> 组件包上限：平台已放宽到 **128 MiB**（`components.rs` 的 `MAX_PACKAGE_BYTES`），
> 足够 GeoLite2/DB-IP City 压缩包；若更大，请先调大该值。

## 3. 上传组件（面板方式）

进入「下载管理」→「组件」→「上传组件」，按下表填写：

| 字段 | 值 |
|------|----|
| 组件 ID | `ip2region-v4` / `ip2region-v6` / `dbip-city` / `geolite2-asn` |
| 标题 | 如 `ip2region IPv4 2026.10` |
| 分类 | **`ip-database`** |
| 版本 | `2026.10`（建议用数据月份，客户端按数字段比较） |
| 目标平台 | **`any`** |
| 说明 | 可选，写数据来源/许可 |
| 包文件 | `dist/{id}-{version}-any.zip` |

上传后确认组件为「已上架」（未上架不会进入公开目录）。

## 4. 上传组件（curl 方式，可选）

```powershell
$base = 'http://127.0.0.1:5502'
# 1) 登录拿 token
$body = '{"user":"admin","password":"admin"}'
[IO.File]::WriteAllText("$env:TEMP\login.json", $body, (New-Object Text.UTF8Encoding($false)))
curl.exe -s -X POST "$base/api/login" -H 'Content-Type: application/json' --data-binary "@$env:TEMP\login.json" -o "$env:TEMP\login-resp.json"
$token = (Get-Content "$env:TEMP\login-resp.json" -Raw | ConvertFrom-Json).data.token

# 2) 上传（zip 作为原始 body）
curl.exe -s -X POST "$base/api/componentUpload?componentId=ip2region-v4&title=ip2region%20IPv4&category=ip-database&version=2026.10&target=any&notes=ip2region" `
  -H "Authorization: Bearer $token" -H 'Content-Type: application/zip' `
  --data-binary "@dist\ip2region-v4-2026.10-any.zip" -o "$env:TEMP\upload-resp.json"
Get-Content "$env:TEMP\upload-resp.json" -Raw

# 3) 取组件源 Ed25519 公钥（填给客户端）
curl.exe -s "$base/api/componentInfo" -H "Authorization: Bearer $token" -o "$env:TEMP\info-resp.json"
(Get-Content "$env:TEMP\info-resp.json" -Raw | ConvertFrom-Json).data.pubkey
```

## 5. 业务系统启动时加载（零业务配置）

业务系统只需要一行：

```rust
# fn main() -> Result<(), pek_ip::Error> {
let locator = pek_ip::IpLocator::load_default()?;
# Ok(()) }
```

平台地址等配置在 **`Config/Ip.toml`**（库首次运行自动生成；平台地址固定，只改 `StoreUrl` 即可）：

```toml
# Config/Ip.toml
StoreUrl = "https://p.sc8.fun"   # 本地调试可改 http://127.0.0.1:5502
PubKey = ""                      # 第 4 步拿到的公钥；非空 = 强制验签
DataDir = "data"
CacheDir = "Data/IpCache"
StateDir = "Data/IpClient"
AutoEnroll = true
DownloadV6 = false
```

> 临时切换平台地址也可用环境变量 `PEK_IP_STORE_URL`；`clientId`/`installId` 由库自动生成，
> 业务系统不需要配置任何身份。

行为：

1. 先查 `data/ip2region_v4.xdb`、`data/ip2region_v6.xdb`、`data/*.mmdb`；
2. 本地缺失 → 查本地缓存 `{cache_dir}/{id}/{version}/`；
3. 缓存也没有 → 请求 `{store_url}/components/catalog.json`，验签后**自动注册/申请票据**并下载、SHA-256 校验、解压安装；
4. 本地缺失/网络失败：`allow_partial=true`（默认）跳过，应用继续启动（可用 `has_china_db()`/`has_world_db()` 判断）；**授权被拒/签名错误不会被跳过**（会明确报错）；
5. `allow_partial=false` 时缺数据直接报错，适合必须要有 IP 库的服务。

> **首次下载会自动注册**：`AutoEnroll=true` 时库在 `StateDir` 生成 `install.id` + Ed25519 私钥，
> 向平台 `/api/ipdb/enroll` 注册。平台命中 `AutoApproveCidrs` 时自动批准；否则管理员在
> 平台「IP 授权」页点「批准」后，业务系统下一次下载即可通过。业务系统不需要配置 clientId 或任何密钥。

## 6. 更新数据

1. 覆盖 `data/` 中的文件（或 `scripts/fetch-data.ps1 -Force`）；
2. 用**新版本号**重新打包上传（同 id+版本+平台不允许重复）；
3. 业务系统重启或调用 `ensure_updated` 即会自动下载新版本；旧版本默认保留 1 份（`keep_versions` 可调）。

## 7. 验证

```powershell
# 组件目录（公开）
curl.exe -s http://127.0.0.1:5502/components/catalog.json | Out-String
# 签名（公开）
curl.exe -s http://127.0.0.1:5502/components/catalog.json.sig | Out-String
```

```powershell
# 本地示例：先删缓存，确认首次会下载
Remove-Item -Recurse -Force .\Data\IpCache -ErrorAction SilentlyContinue
cargo run --features manager --example startup -- 114.114.114.114 8.8.8.8
```

> 直接访问 `GET /components/{ip 包}.zip` 而不带票据会返回 **403**（仅 `category=ip-database` 的包）；
> 数据库驱动等其它组件仍为公开下载。

## 8. 防滥用说明（重要）

平台 v0.1.42 起已实现「License + 短期票据」：

- 平台「配置」页设置 `AutoApproveCidrs`（如 `127.0.0.1/32,10.0.0.0/8,192.168.0.0/16`）；
- 业务系统首次启动自动注册；命中 CIDR 自动批准，否则在「IP 授权」页手动批准；
- 每次下载前客户端用私钥签票据，平台验签、查每日配额（默认 20 次/天）、5 分钟过期、nonce 防重放；
- 管理员可随时吊销客户端；下载时实时回查状态，吊销立即生效；
- 平台部署在 Nginx 等反向代理之后时，勾选「信任反向代理头」或设置 `IpdbTrustProxy = true`，以便按真实客户端 IP 做 CIDR 判定。

详见 [`下载授权协议.md`](下载授权协议.md)。
