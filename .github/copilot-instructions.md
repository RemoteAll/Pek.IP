# Pek.IP · 项目指令

> 全项目通用规则见用户级《跨项目接续协议》（本机 `%APPDATA%\Code\User\prompts\跨项目接续协议.instructions.md`；模板在 `tools/continuity/templates/`）。
> **开工先读（存在则读，按序）**：`.github/copilot-instructions.md` → `docs/项目接续指南.md` → `docs/用户档案.md` → `docs/项目记忆.md` → `docs/决策记录.md` 最新节。

## 每轮收尾（务必）

① `docs/决策记录.md` 新增一节（**含用户原话**）；② `tools/continuity/sync-memory.ps1` 同步记忆；③ 跑回归验证并如实汇报。

## 本项目要点

- **库 crate**：`pek-ip`，无独立二进制；对外主入口 `IpLocator` / `IpDbManager`。
- **数据文件在仓库 `data/`，但不打包进 crate/二进制**（`Cargo.toml` 的 `exclude = ["data/**", ...]`）。
- **国内 + 国外**：ip2region xdb v3（v4/v6）+ MaxMind/DB-IP MMDB；合并逻辑在 `locator.rs`。
- **按需下载**：消费 Pek.RPanlServer 组件源 `/components/catalog.json`（Ed25519 + SHA-256 + zip）。
- **启动加载**：`IpLocator::load_default()` 读取 `Config/Ip.toml`（平台地址 `StoreUrl` 默认固定 `https://p.sc8.fun`），本地 `data/` 优先，缺失再下载。
- **业务系统零身份配置**：`clientId`/`installId` 由库自动生成；业务系统不改身份配置。
- **下载授权（防 PCDN）**：`AutoEnroll=true` 时 `src/auth.rs` 自动生成 installId + Ed25519 密钥，调用 `/api/ipdb/enroll|ticket`；平台 `src/ipdb.rs` + 面板「IP 授权」页负责审批/配额/票据；仅 `category=ip-database` 包要求票据。旧平台 404 自动回退公开下载，403/401 不回退。
- 测试脚本：`scripts/_e2e-auth.ps1`（自动批准）、`_e2e-auth-manual.ps1`（待批准/配额/吊销/包策略）、`_e2e-startup.ps1`（真实数据启动加载）。
- 构建：`cargo check` / `cargo test`；全特性：`cargo test --all-features`；格式：`cargo fmt`。
- 改 `data/` 中文件或新增数据格式时，同步 `scripts/fetch-data.ps1`、`scripts/pack-ipdb.ps1` 与文档。
