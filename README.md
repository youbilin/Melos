# Melos

一个轻量的 Navidrome / OpenSubsonic 桌面客户端，Rust + GPUI 编写，Windows 优先，附带 TUI 模式。

> 状态：**v1.0 开发与验收阶段**。浏览、搜索和播放闭环已接通，正在收尾 Windows 分发与稳定性验证。详见 [实施计划](docs/01-implementation-plan.md) 与 [验收记录](docs/03-acceptance.md)。

实际功能清单与后续优先级见 [项目进展](docs/04-project-status.md)。已实现的入口与尚未验收的运行场景分别记录。

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
  melos-player/   symphonia + cpal 播放引擎、HTTP Range 与断流恢复
  melos-core/     队列状态机、设置与凭据保存（无 UI 依赖）
  melos-app/      GPUI 桌面前端（唯一依赖 gpui 的 crate）
  melos-tui/      ratatui 终端前端（复用队列、歌词与播放器）
docs/          调研与技术方案
spikes/        独立技术验证工程（不进入 workspace）
```

桌面与终端前端组合 `melos-core`、`melos-player` 与 `melos-api`；底层 crate 不依赖 UI 工具包。

## 构建

`rust-toolchain.toml` 固定 Rust 1.99.0；GPUI 最低要求 1.95。安装 MSVC Build Tools 和 Windows SDK 后构建：

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
- [x] **M1 `melos-api`**：认证、浏览、搜索、歌单读写、流 URL 能力矩阵、错误映射、重试
- [x] **M2 `melos-player`**：文件/HTTP 源、解码、gapless、seek、音量、位置；队列编辑可替换已预取音频
- [x] **M3 `melos-app` 外壳**：GPUI 窗口、侧边栏、沉浸式背景、播放条、队列抽屉
- [x] **M4 数据视图**：连接页、真实专辑/艺人/专辑详情、搜索（IME 输入框）、封面加载
- [x] **M5 核心播放**：流播放、队列、播放控制、gapless、scrobble
- [x] M5 收尾：进度/音量拖动、专辑收藏与失败回滚、Windows SMTC 原生窗口绑定
- [x] 播放栏封面：播放/自动切歌主动加载，歌曲与专辑封面 ID 回退，当前播放优先加载
- [x] 队列编辑：下一首播放、歌曲/整张专辑追加、移除、拖动/按钮排序、清空待播；保留当前歌曲和暂停状态
- [x] 随机播放、单曲/列表循环，与预取和队列编辑联动；模式随设置保存
- [x] 播放快捷键，输入框/中文输入法组合保护
- [x] 曲库专辑分页与可见行渲染、按需封面、失败重试；磁盘封面缓存按服务器/账号隔离
- [x] 桌面歌单：浏览/播放、从队列新建/追加、移除歌曲、重命名、确认删除与权限错误提示
- [x] 歌词：封面卡片、同步行居中、高亮与点击跳转、多版本切换、纯文本/LRC 与旧接口回退
- [x] 侧栏收起与图标导航、随机/循环图标、艺人 → 专辑的逐层返回；移除操作说明小字
- [x] 设置与持久化：音质、音量、连接信息、DPAPI 密码保护与密码掩码
- [x] 仓库自带音频测试素材，多格式解码、seek、gapless 与断流恢复回归测试
- [x] 音频输出错误反馈与手动重连；保留当前歌曲、待播队列和进度，重连后保持暂停
- [x] M6 首版：release 优化、应用图标/版本信息、Windows 绿色版 ZIP 与校验和
- [x] GitHub Actions：格式、测试、Clippy、底层依赖检查、Windows 构建与私有构建产物
- [ ] 发布验收：真实媒体键、80 分钟真实服务稳定性、完整应用性能目标
- [x] TUI 首版：专辑分页、歌单浏览/播放、歌曲搜索、队列编辑、随机/循环、同步/纯文本歌词

Opus、WMA、APE、DSD 等无法本地解码的格式显式请求服务端转为 MP3。桌面歌曲与队列长列表已使用可见行渲染。歌词入口在播放栏右侧，页面只保留封面与歌词；同步行自动居中，纯文本不编造时间轴，多个版本通过封面下的图标切换。

终端版先复用桌面保存的连接信息与密码，在交互终端中运行 `melos-tui.exe`（或开发时 `cargo run -p melos-tui --locked`）。`--help` 显示按键；1/2/3/4 切换专辑、歌单、队列、歌词，`/` 搜索歌曲，Enter 打开/播放，空格暂停，n/p 切歌，方向键 seek 或选条目。歌单编辑仍使用桌面；终端首版暂不支持艺人/专辑搜索、媒体键和独立连接表单。

## Windows 绿色版

```powershell
./packaging/build-windows.ps1
```

产物在 `output/release/`，包含 ZIP 和 SHA256。程序要求 Windows 10 1903+ / Windows 11（x64）以及 Microsoft Visual C++ 2015–2022 Redistributable（x64）。当前 0.1.6 为开发预览，新增侧栏收起、播放模式图标、歌词封面卡片与居中滚动，修复艺人详情逐层返回及 FLAC 曲尾轻微时长误差。队列编辑、分页曲库、歌单、音频输出重连及系统媒体封面均已接入。真实服务中的《突然好想你》全文件解码和最后十秒静音播放已通过，完整格式矩阵仍按验收记录分别追踪。

音频设备断开后，在桌面设置页点“重新连接音频输出”，或在终端按 `e`。重新打开默认设备并保留歌曲、队列和播放位置；确认设备后按播放继续。

```powershell
# 无人值守的窗口初始化、SMTC 与空闲内存粗测
./packaging/measure-windows.ps1
# 显式运行 30 秒断流恢复验收
cmd /c '"C:\Program Files (x86)\Microsoft Visual Studio\18\BuildTools\VC\Auxiliary\Build\vcvars64.bat" && cargo test -p melos-player stream_recovers_after_thirty_seconds_offline -- --ignored'
# 静音的本地素材/默认设备长测，记录进度漂移、内存、队列与重连检查
cmd /c '"C:\Program Files (x86)\Microsoft Visual Studio\18\BuildTools\VC\Auxiliary\Build\vcvars64.bat" && cargo run -p melos-player --example soak --release --locked -- --seconds 4800 --report output/playback-soak.json'
# 使用桌面已保存连接，静音检查真实服务器格式样本的播放/暂停/seek；不改设置或服务端数据
cmd /c '"C:\Program Files (x86)\Microsoft Visual Studio\18\BuildTools\VC\Auxiliary\Build\vcvars64.bat" && cargo run -p melos-core --example server_check --release --locked -- --albums 30 --play-seconds 30 --report output/server-check.json'
# 使用保存的真实连接，检查指定歌曲的全文件解码与最后十秒播放；报告不包含认证 URL
cmd /c '"C:\Program Files (x86)\Microsoft Visual Studio\18\BuildTools\VC\Auxiliary\Build\vcvars64.bat" && cargo run -p melos-core --example track_check --release --locked -- "歌曲全名" output/track-check.json'
```

项目保持私有；完成并验证的改动按批次提交、推送，见 [项目工作约定](AGENTS.md)。
