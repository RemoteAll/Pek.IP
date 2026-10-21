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
- **启动加载**：`IpLocator::load_or_download(...)` 优先本地 `data/`，缺失再下载。
- 构建：`cargo check` / `cargo test`；全特性：`cargo test --all-features`；格式：`cargo fmt`。
- 改 `data/` 中文件或新增数据格式时，同步 `scripts/fetch-data.ps1`、`scripts/pack-ipdb.ps1` 与文档。
