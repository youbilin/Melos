# ADR-001：GPUI UI 栈选择

- 状态：已接受
- 日期：2026-11-11
- 决策人：项目所有者
- 相关：[Phase 0 报告](../02-phase0-report.md)、[调研方案](../00-research.md) §3.4

---

## 背景

Phase 0 验证了两条 GPUI 技术栈：

| | A | C |
|---|---|---|
| 组成 | `gpui-unofficial =1.28.0-pre` + `gpui-platform-gpui-unofficial` | `gpui-kit =0.9.1`（内含 `gpui-pre =0.3.12`、`gpui-component =0.9.1`） |
| 组件库 | 无 | 75+ 组件 |
| 依赖包数 | 655 | 801 |
| 中文 IME | 通过 | 通过 |
| debug exe | 24.7 MB | 51.1 MB |
| debug 工作集 | ~68 MB | ~91 MB |

## 决策

**采用 A：`gpui-unofficial`，自建最小控件集。**

## 依据

1. **体积差距已超过阈值。** A 的 release exe 为 10.1 MB；C 的 debug 是 A 的 2.07 倍。按 A 的 debug→release 比例（2.45×）估算，C 的 release 约 21 MB，相对 A 增加约 11 MB，超过计划中「+8MB 则选 A」的规则。
2. **定位优先。** 10 MB 级别的原生客户端是本项目最直观的差异化，值得用开发时间换。
3. **组件需求有限。** 播放器实际只需要：输入框、按钮、图标按钮、滑块、菜单、对话框、Toast。GPUI 自带虚拟列表、滚动、布局、文本渲染。
4. **输入框已有验证实现。** 官方 `examples/input.rs`（920 行，Apache-2.0）已实测通过中文 IME，可直接内联并保留声明。
5. **依赖面更小。** 655 vs 801，许可证审计与供应链风险都更低。

## 后果

**正面**
- release 体积 10.1 MB，空闲工作集 54 MB，均在目标内
- 不受组件库 API 与主题体系约束，沉浸式视觉可以完全自定义
- 依赖树更小，构建更快（2m17s vs 更长）

**负面**
- 需自建 6–7 个基础控件，预计 1–1.5 周
- `gpui-unofficial` 由单人维护，README 明确写着「Currently needs a maintainer」

## 缓解措施

1. **`cargo vendor` 冻结依赖**：即使上游停止发布，锁定版本仍可构建。
2. **控件集中管理**：全部放在 `melos-app/src/ui/`，与业务视图隔离。
3. **保留切换路径**：`melos-core` / `melos-api` / `melos-player` 不依赖 GPUI；若控件工作量失控，切到 C 只需重写 `melos-app` 的视图层。
4. **时间盒**：控件阶段超过 2 周仍未收敛，重新评估 C。

## 备选方案

**C（gpui-kit）**：组件齐全、接入成本极低、由 Longbridge 商业产品背书，但体积与依赖更重。保留为 fallback，不删除 spike 代码。

## 备注

C 的 release 构建因释放 CPU 被中止，未取得最终数字；上述 21 MB 为按 A 的压缩比例推算，仅用于决策，不作为精确结论。若后续需要严格对比，可单独补测。
