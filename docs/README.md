# docs 索引

r-mapserver / r-manager / gis-wt 项目的设计、分析与规划文档。按性质分三类维护：

## 活动 / 长期文档

| 文档 | 说明 |
|---|---|
| [architecture.md](architecture.md) | 架构总览（数据引擎 + 协议层 + 服务面/管理面）——长期维护 |
| [geoserver-org-plan-2026-11-01.md](geoserver-org-plan-2026-11-01.md) | r-mapserver / r-manager 对齐 GeoServer 组织逻辑（Workspace→Store→Resource→PublishedLayer）的开发计划与验收走查 |
| [ai-数据底座开发计划-2026-11.md](ai-数据底座开发计划-2026-11.md) | **面向 AI 的空间数据底座开发计划（定稿）**：三仓职责边界 + MCP 工具网关拓扑 + P0–P3 工作项 |

## 历史分析汇编

| 文档 | 说明 |
|---|---|
| [devlog-2026-10.md](devlog-2026-10.md) | 2026-10 开发分析与决策汇编——由当日 8 份定点报告按日期合并整理而来，正文原样保留 |

`devlog-2026-10.md` 合并来源（原独立文件已归档入此汇编）：

1. r-mapserver 二轮修复报告（配置 P2 + 性能 P1/P2，2026-10-21）
2. 缓存机制 moka → foyer（2026-10-24）
3. 能力评估（2026-10-24）
4. 与 GeoServer 服务对比（2026-10-24）
5. WFS 与 OGC API 实现（2026-10-24）
6. PostGIS 能力分析（2026-10-25）
7. PostGIS 服务实现（2026-10-25）
8. 管理面范围分析（2026-10-26）

## 规划依据（PDF）

| 文档 | 说明 |
|---|---|
| 面向AI的空间数据底座_产品规划文档(1).pdf | 产品规划：7 层能力架构 + 38 个 AI 工具清单 + 分阶段路线 |
| Rust空间数据Server-Manager-AI编程时代发展前景分析.pdf | 可行性论证：AI 编程时代 Rust GIS 前景 + Open-Core 商业模式 |

> 约定：新的**活动/长期文档**单独成篇；一次性、带日期的分析/走查报告统一并入 `devlog-<年-月>.md`，避免 docs 目录碎片化。
