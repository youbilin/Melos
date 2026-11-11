# Melos

一个轻量的 Navidrome / OpenSubsonic 桌面客户端，Rust + GPUI 编写，Windows 优先，附带 TUI 模式。

> 状态：**骨架阶段**。UI 与音频引擎尚未接入，详见 [调研方案](docs/00-research.md) 与 [实施计划](docs/01-implementation-plan.md)。

## 设计约束

| 项 | 目标 |
|---|---|
| 二进制 | < 25MB（strip 后） |
| 冷启动 | < 1.0s |
| 空闲内存 | < 80MB |
| 依赖 | 不引入 WebView / MPV / GStreamer |
| 许可证 | GPL-3.0 |

## 目录结构

```
crates/
  melos-api/      Subsonic / OpenSubsonic 客户端（认证、DTO、错误）
  melos-player/   播放引擎抽象（symphonia + cpal 实现待接入）
  melos-core/     领域逻辑：队列、缓存、设置、scrobble（无 UI 依赖）
  melos-app/      GPUI 桌面前端（唯一依赖 gpui 的 crate）
  melos-tui/      ratatui 终端前端（v1.1，复用 melos-core）
docs/          调研与技术方案
spikes/        独立技术验证工程（不进入 workspace）
```

依赖方向严格单向：`melos-app → melos-core → melos-player → melos-api`，`melos-tui → melos-core`。

## 构建

本机默认的 `stable` 工具链是 1.92，低于 GPUI 社区版要求的 1.95；已安装的 1.99.0 可用：

```powershell
rustc +1.99.0-x86_64-pc-windows-msvc --version
```

MSVC 链接器需要先进入开发者环境（否则 Git Bash 的 `link.exe` 会被误用）：

```powershell
cmd /c '"C:\Program Files (x86)\Microsoft Visual Studio\18\BuildTools\VC\Auxiliary\Build\vcvars64.bat" && cargo check --workspace'
```

运行测试：

```powershell
cmd /c '"C:\Program Files (x86)\Microsoft Visual Studio\18\BuildTools\VC\Auxiliary\Build\vcvars64.bat" && cargo test --workspace'
```

## 当前进度

- [x] 桌面客户端生态调研与定位
- [x] workspace 骨架、API 认证、队列状态机
- [x] Phase 0（UI）：窗口 + 中文 IME 验证完成，选定 `gpui-unofficial`（[ADR-001](docs/adr/0001-ui-stack.md)）
- [x] Phase 0（音频）：本地解码 + WASAPI 输出 + MP3 接缝验证完成
- [x] **M1 `melos-api`**：认证、浏览、搜索、歌单、流 URL 能力矩阵、错误映射、重试（11 个测试）
- [x] **M2 `melos-player`**：文件/HTTP 源、解码、gapless、seek、音量、位置（7 个测试）
- [x] **M3 `melos-app` 外壳**：GPUI 窗口、侧边栏、沉浸式背景、播放条、队列抽屉
- [x] **M4 数据视图**：连接页、真实专辑/艺人/专辑详情、搜索（IME 输入框）、封面加载
- [x] **M5 核心播放**：流播放、队列、播放控制、gapless、scrobble
- [ ] M5 其余：进度/音量拖动、Star、SMTC、设置持久化
- [ ] M6：打包分发
- [ ] Phase 2：歌词、歌单、TUI
