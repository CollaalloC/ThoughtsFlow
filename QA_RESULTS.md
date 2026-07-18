# ThoughsFlow 原型验证记录

> 验证日期：2026-07-18
> 验证对象：UI Prototype 0.1

## 结论

三个设计方向均可运行，核心交互已覆盖，桌面端没有横向溢出，浏览器控制台没有报错。推荐默认方向 `Focus` 已验证“选择具体回答版本 → 从该回答创建分支 → 检查上下文 → 发送”的完整路径。

## 自动检查

- `npm run build`：通过（TypeScript `--noEmit` + Vite production build）
- Chromium / Google Chrome：通过
- 视口：`1440 × 900`、`1280 × 800`
- 控制台错误：0
- 页面横向溢出：0
- 原型切换：`?variant=focus|canvas|trace` 均可用

| 方向 | 关键路径 | 结果 |
|---|---|---|
| Focus | 回答 A/B 切换、精确回答分支、Context Inspector、pin / exclude、发送 | 通过 |
| Canvas | 节点选择、回答端口切换、缩放、Reveal context、新分支入图 | 通过 |
| Trace | 上下文清单 include / exclude、本地/云端提供商、Next send / Run history、事件展开 | 通过 |

## 截图证据

- `output/prototype-screenshots/focus-default.png`
- `output/prototype-screenshots/focus-branch.png`
- `output/prototype-screenshots/canvas-default.png`
- `output/prototype-screenshots/canvas-branch.png`
- `output/prototype-screenshots/trace-default.png`
- `output/prototype-screenshots/trace-history.png`

## 当前验证边界

本轮是前端交互原型验证，不包含真实模型调用、本地数据库、跨设备同步、移动端适配与辅助技术专项测试；这些应在工程原型阶段单独验收。
