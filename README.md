# Melos

面向 [Navidrome](https://www.navidrome.org/) / OpenSubsonic 的原生音乐客户端，提供 Windows 桌面界面和终端界面（TUI）。使用 Rust 编写，界面层为 GPUI，播放引擎基于 symphonia + cpal，不依赖 WebView、MPV 或 GStreamer。

- 当前版本：**0.1.12（技术预览）**
- 支持平台：Windows 10 1903+ / Windows 11（x64）
- 许可证：GPL-3.0-or-later

> **发布状态**：0.1.12 的主要功能已接通并通过自动化检查，可以作为公开测试版使用；但按项目验收标准，仍有四项未完成（真实媒体键、真实服务 80 分钟连续播放、冷启动 < 1 秒、完整界面交互矩阵），因此不标记为 1.0 正式版。已验证与待验收的范围见[验收记录](docs/03-acceptance.md)。

## 功能

### 播放与队列
- 流式播放，本地解码 WAV、FLAC、MP3、AAC/ALAC、Vorbis；Opus、WMA、APE、DSD 等格式由服务端转码为 MP3
- 无缝合辑（gapless）、精确 seek、预取、断流恢复、HTTP Range 续传
- 单曲 / 列表循环、全库随机（队列播完自动续抽）、随机/循环状态随设置保存
- 队列编辑：下一首播放、追加歌曲或整张专辑、移除、拖动/按钮排序、清空待播，编辑不打断当前音频
- 播放进度与音量拖动；音频设备断开后可手动重连，保留歌曲、队列和位置

### 浏览与曲库
- 专辑、艺人、歌单、全部歌曲、搜索、收藏
- 全部歌曲按艺人 → 专辑 → 碟号 → 音轨排序，分页加载
- 专辑/歌单歌曲列表使用可见行渲染，长列表不卡顿；封面按需加载并做磁盘缓存
- 歌单：浏览/播放、从队列新建或追加、移除歌曲、重命名、确认删除

### 歌词
- 点击播放栏封面进入歌词页，再点封面返回原页面
- 同步歌词按 240 ms 缓动居中；点击行跳转；支持多版本切换、纯文本与 LRC
- 纯文本歌词不编造时间轴；旧版歌词接口自动回退

### 桌面体验
- 可收起侧栏；沉浸式背景；队列抽屉
- Windows 系统媒体控制（SMTC）与封面
- 快捷键：空格播放/暂停，←/→ seek 5 秒，Ctrl+←/→ 上一首/下一首，Ctrl+↑/↓ 音量，Ctrl+S/R/Q 随机/循环/队列，Ctrl+F 搜索

### 终端（TUI）
- 可独立登录，不依赖桌面版；也可以沿用桌面保存的音乐源
- 音乐源管理：添加、切换、删除（每源独立密码）
- 专辑、歌单、全部歌曲、歌曲搜索、队列编辑、歌词
- 同步歌词居中；Esc 按页面历史逐级返回

### 设置
- 多音乐源：添加只保存，显式切换时才连接
- 音质：原始 / 128 / 192 / 320 kbps（转码由服务端完成）
- 音量、随机、循环持久化；密码使用 Windows DPAPI 加密

## 系统要求

- Windows 10 1903+ 或 Windows 11（x64）
- [Microsoft Visual C++ 2015–2022 Redistributable (x64)](https://aka.ms/vs/17/release/vc_redist.x64.exe)

## 下载与运行

| 包 | 内容 |
|---|---|
| `melos-0.1.12-windows-x64.zip` | 桌面版 + 终端版 |
| `melos-tui-0.1.12-windows-x64.zip` | 仅终端版 |

产物位于 `output/release/`，每个 ZIP 旁附带 `.sha256` 校验文件；当前版本的哈希与文件大小记录在[验收记录](docs/03-acceptance.md)。

**桌面版**：解压后运行 `melos.exe`，填写服务器地址、用户名和密码，点击“连接”。

**终端版**：在 Windows Terminal 或 PowerShell 中运行 `melos-tui.exe`。没有可用连接时停在“音乐源”页：

1. 按 `6` 进入音乐源（首次启动即在此页）
2. 按 `a`，依次输入服务器、用户名、密码，Enter 提交并连接
3. 已有源时用 `↑/↓` 选择、`Enter` 连接；`d` 删除非当前源

## 终端按键

| 键 | 作用 |
|---|---|
| `1` / `2` / `3` / `4` / `5` / `6` | 专辑 / 歌单 / 队列 / 歌词 / 全部歌曲 / 音乐源 |
| `/` | 搜索歌曲（Enter 提交，Esc 取消） |
| `↑` `↓` 或 `k` `j` | 选择条目 |
| `Enter` | 打开专辑/歌单，播放歌曲，连接音乐源 |
| `空格` | 播放 / 暂停 |
| `n` / `p` | 下一首 / 上一首 |
| `←` / `→` | 后退 / 前进 5 秒 |
| `+` / `-` | 音量 |
| `s` / `r` | 随机 / 循环 |
| `a` / `i` | 加入队列 / 下一首播放（歌曲页） |
| `d` / `J` / `K` / `D` | 移除 / 下移 / 上移 / 清空待播（队列页） |
| `L` | 加载更多（专辑页、全部歌曲页） |
| `t` | 切换歌词版本（歌词页） |
| `e` | 重新连接音频输出 |
| `Esc` | 取消输入 / 返回上一级 |
| `q` 或 `Ctrl+C` | 退出 |

终端版暂不支持歌单编辑、艺人/专辑搜索和系统媒体键，这些功能请使用桌面版。

## 配置与隐私

- 设置保存在 `%APPDATA%\melos\settings.json`，包含音乐源列表、音质、音量与播放模式
- 密码用 Windows DPAPI 加密，绑定当前 Windows 用户；不写入明文
- 设置 `MELOS_CONFIG_DIR` 可指定独立配置目录，桌面版与终端版可完全隔离
- 除连接你指定的服务器外，不上传数据、不包含遥测

## 从源码构建

工具链固定在 `rust-toolchain.toml`（Rust 1.99.0，MSVC 目标）。需要 Visual Studio Build Tools（C++ 工作负载）和 Windows SDK。

```powershell
# 进入 MSVC 环境后
cargo build --release --locked -p melos-app -p melos-tui
```

常用检查：

```powershell
cargo fmt --all --check
cargo test --workspace --locked
cargo clippy --workspace --all-targets --locked -- -D warnings
```

> Git Bash 直接调用 `cargo` 时可能误用其他 `link.exe`。请通过 `cmd /c "\"...\vcvars64.bat\" && cargo ..."` 或已导入 MSVC 环境的 PowerShell 运行。仓库的 `packaging/build-windows.ps1` 会自动定位 MSVC。

## 打包

```powershell
./packaging/build-windows.ps1
```

生成桌面+终端 ZIP、仅终端 ZIP 及各自的 SHA256。脚本会校验二进制资源版本与工作区版本一致。

## 架构

```
crates/
  melos-api/      Subsonic / OpenSubsonic 客户端（认证、DTO、流 URL、错误映射）
  melos-player/   symphonia 解码 + cpal 输出、预取、seek、断流恢复
  melos-core/     队列状态机、歌词、排序、设置与 DPAPI 凭据（无 UI 依赖）
  melos-app/      GPUI 桌面端（唯一依赖 gpui 的 crate）
  melos-tui/      ratatui 终端端（复用 core / player / api）
```

领域层与播放器不依赖 GPUI；CI 会检查底层 crate 的依赖图。

## 质量与验证

- 94 个自动化测试通过（1 个 30 秒断流长测试显式忽略），覆盖解码、seek、gapless、断流恢复、队列、随机、设置迁移与 TUI 交互
- `cargo fmt`、`cargo clippy -D warnings` 通过
- 真实服务验证：原始 FLAC 整曲解码与跨曲播放、40 首随机样本完整解码、转码流曲尾修复前后各 3 首对比、终端隔离配置登录并加载 80 张专辑
- 发布包：桌面 13.64 MB、终端 5.18 MB，空闲内存约 47–48 MB

尚未完成的人工验收：

- [ ] 真实媒体键（键盘媒体键与系统面板）
- [ ] 真实服务连续播放 80 分钟（内存与漂移）
- [ ] 冷启动 < 1.0 秒（首次隐藏初始化约 1.4–2.5 秒，后续约 0.25 秒）
- [ ] 拖动进度/音量、中文输入、缩放与长列表的完整界面矩阵

## 已知限制

- 本地只解码 WAV/FLAC/MP3/AAC/ALAC/Vorbis，其他格式依赖服务端转码
- 随机播放目前从整个曲库抽歌；按专辑或艺人限定范围尚未实现
- 终端版没有歌单编辑、艺人/专辑搜索与媒体键
- 不支持 Windows 10 1903 之前的系统

## 文档

- [实施计划](docs/01-implementation-plan.md)
- [验收记录](docs/03-acceptance.md)
- [项目进展](docs/04-project-status.md)
- [UI 技术选型](docs/adr/0001-ui-stack.md)

## 许可证

GPL-3.0-or-later。第三方组件声明见 `packaging/THIRD_PARTY_NOTICES.md`。
