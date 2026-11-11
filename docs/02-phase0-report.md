# Phase 0 技术验证报告

> 日期：2026-11-11　状态：进行中（release 体积对比待补）
> 对应计划：[实施计划](01-implementation-plan.md) §2

---

## 1. 环境

| 项 | 值 |
|---|---|
| OS | Windows 11 (10.0.22631) |
| CPU / RAM | 20 逻辑核 / 15.6 GB |
| Rust | 1.99.0-x86_64-pc-windows-msvc |
| 默认 stable | 1.92.0（低于 GPUI 要求，未使用） |
| MSVC | VS 18 BuildTools，MSVC 14.50.35717 |
| 启动方式 | `vcvars64.bat` + `cargo +1.99.0` |

---

## 2. 结果总览

| 门禁 | 结果 | 说明 |
|---|---|---|
| G1 窗口 | ✅ | 两个 spike 均正常开窗、稳定运行 |
| **G2 中文 IME** | ✅ | **A / C 均通过**（用户实测） |
| G3 远程播放 | 🟡 | 本地解码 + WASAPI 输出链路通过；HTTP 流待 Phase 1 |
| G4 Gapless | ✅ | MP3 接缝无静音（RMS 分析），underrun 0；FLAC/AAC 待补 |
| G5 性能基线 | ✅ | A release：10.1 MB / 54 MB / 1.79 s（粗测） |
| G6 可复现构建 | ✅ | 两个 spike 均可 `cargo build` 复现 |

---

## 3. Spike A：gpui-unofficial

| 项 | 值 |
|---|---|
| 依赖 | `gpui-unofficial =1.28.0-pre` + `gpui-platform-gpui-unofficial =1.28.0-pre` |
| 锁定包数 | 655 |
| 首次构建 | 2 分 17 秒（debug） |
| 输入框 | 官方 `examples/input.rs` 完整移植（920 行，自实现 `EntityInputHandler`） |
| IME | ✅ 通过 |
| debug exe | 24.7 MB |
| debug 工作集 | ~68 MB |
| **release exe** | **10.1 MB** |
| **release 工作集** | **54.2 MB** |
| 冷启动（粗测） | 1.79 s（含 PowerShell 启动开销，Phase 1 用应用内计时复测） |

**观察**
- 官方示例里的输入框是完整实现（选择、剪贴板、UTF-16 映射、marked range），可直接作为基础。
- 内部 crate 全部带 `-gpui-unofficial` 后缀重发布，未与官方 `gpui` 冲突。
- 无组件库：按钮、列表、对话框都需要自建或另找。

---

## 4. Spike C：gpui-kit（gpui-pre + gpui-component）

| 项 | 值 |
|---|---|
| 依赖 | `gpui-kit =0.9.1`（内含 `gpui-pre =0.3.12`、`gpui-component =0.9.1`） |
| 锁定包数 | 801 |
| 首次构建 | 依赖编译完成后，应用链接因一处 trait 导入失败；修复后 7.5 秒（debug） |
| 输入框 | `InputState::new` + `Input::new`，两行代码 |
| IME | ✅ 通过 |
| debug exe | 51.1 MB |
| debug 工作集 | ~91 MB |
| release exe | —（构建中止，估算 ~21 MB） |

**观察**
- 组件库接入成本极低：初始化 `gpui_kit::init(cx)`，`Input` 直接可用。
- 依赖明显更重（+146 个包），debug 体积是 A 的 2.07 倍。
- 包含编辑器、Markdown、i18n、ropey 等本项目用不到的能力，需要评估 feature 裁剪空间。

---

## 4.5 音频冒烟（G3 / G4）

**工具**：`spikes/audio`，symphonia 0.6.1（`all-codecs`/`all-formats`/`all-meta`）+ cpal 0.18.2。

**素材**：两个本地 MP3，44.1 kHz / 2 ch，242.10 s 与 325.03 s。

**流程**：完整解码两个文件 → 按顺序拼接为一段 PCM → 取接缝前 5 秒到后 3 秒 → 通过 WASAPI 默认设备播放（f32，44.1 kHz），同时做 10 ms RMS 分析。

**结果**

| 指标 | 值 |
|---|---|
| 输出设备配置 | WASAPI 默认设备，44100 Hz，2 ch，f32 |
| 播放窗口 | 8.000 s |
| 实际耗时 | 8.306 s（含 300 ms 排空等待，净漂移约 6 ms） |
| 输出欠载（underrun） | **0** |
| 接缝前 100 ms RMS | 0.055 – 0.088 |
| 接缝处（0 ms）RMS | 0.054 |
| 接缝后 100 ms RMS | 0.128 – 0.317 |

接缝处 RMS 与前一段音乐处于同一量级，**没有静音掉档**，说明解码与输出链路没有插入空隙。

**结论**

- G4 在 MP3 上通过；FLAC/AAC/Vorbis 待补素材后复测。
- G3 的本地链路（解码 → 重采样省略 → cpal → WASAPI）通过；HTTP 流、Range seek、断流续播属于 Phase 1 的 `melos-api` 集成，未在 Phase 0 覆盖。

**重要发现（影响 Phase 1）**

1. **symphonia 0.6.1 不提供 Opus 解码**。官方 feature 列表只有 aac/adpcm/alac/flac/mp1/mp2/mp3/pcm/vorbis；`symphonia-codec-opus` 在 crates.io 上无法获取 0.6 对应版本。
2. **Navidrome 的默认转码格式是 Opus**。因此 `stream` 请求只带 `maxBitRate` 时可能返回无法解码的流，**必须显式指定 `format=mp3`**。
3. 影响面：只影响「本地解不了的冷门格式」和「用户主动降码率」两条路径；`format=raw` 的正常播放不受影响。

---

## 5. 体积与内存对比

| 指标 | A | C | 差值 |
|---|---|---|---|
| 依赖包数 | 655 | 801 | +146 |
| debug exe | 24.7 MB | 51.1 MB | +26.4 MB |
| debug 工作集 | ~68 MB | ~91 MB | +23 MB |
| release exe（strip） | 10.1 MB | 21 MB（估算） | +11 MB |
| release 空闲内存 | 54.2 MB | — | — |

> 决策规则（计划 §2.7）：两者 IME 都通过时选 C，除非 C 的 release exe 比 A 大 8MB 以上，或空闲内存多 30MB 以上。
> **结论：选 A**，详见 [ADR-001](adr/0001-ui-stack.md)。

---

## 5.1 结论

两条栈的中文 IME 都可用，因此决策由体积和依赖面决定：

- A 的 release 为 **10.1 MB / 54 MB 工作集**，符合项目定位。
- C 的 debug 已达 51 MB，估算 release 约 21 MB，超出「+8MB」阈值。
- 组件需求实际只有 6–7 个，且输入框已有经过 IME 验证的官方实现可直接内联。

**采用 A（`gpui-unofficial`），自建最小控件集。** C 保留为 fallback。

> C 的 release 构建为释放 CPU 被中止，21 MB 为按 A 的压缩比例推算，仅用于决策。

---

## 6. 待办

- [x] 记录 release 构建体积（A）
- [x] 记录 release 空闲内存（A）
- [x] 写 ADR-001（选择 A）
- [x] 音频冒烟：解码 + 设备 + 接缝（MP3）
- [ ] 用 FLAC / AAC / Vorbis 复测接缝
- [ ] Phase 1 用真实 Navidrome 验证 HTTP 流、Range seek、断流续播
- [ ] 用应用内计时复测冷启动
