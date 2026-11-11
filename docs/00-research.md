# Melos 调研与技术方案

> 目标：用 Rust + GPUI 做一个轻量的 Navidrome / OpenSubsonic 桌面客户端
> 日期：2026-11-11　状态：待评审

---

## 0. 结论先行

1. **这个赛道已经不空了，但空位仍然存在。** 桌面端有 Feishin（Electron，功能最全）、Sonora（Rust + GPUI，跨音乐源的重型客户端）、Supersonic（Go + GTK + MPV）、Psysonic（Tauri）、Rufin / Tributary（GNOME 原生）、Firmium / Nokkvi（Iced）。**没有一个是"只做 Navidrome、Windows 优先、单文件、80MB 内存以内"的。**
2. **GPUI 可用，但不能裸用 crates.io 的官方 `gpui`。** 官方版本停在 0.2.2（2025-10），落后 Zed 主线近一年。必须锁一个 fork 的具体版本并 vendor 进来，否则依赖冲突会让你痛不欲生。
3. **音频栈推荐 symphonia + cpal 自建引擎，rodio 作为过渡封装。** GPUI 不提供任何音频/系统集成能力，这块 100% 自研。MPV 方案（Feishin / Supersonic 在用）会直接摧毁"轻量"这个卖点。
4. **最大的技术风险不是渲染，是中文输入法。** GPUI 在 Windows 上的 IME 有已知 bug（Zed issue #56327：WM_KEYDOWN 与 WM_CHAR 双路径重复插入、DirectWrite 崩溃）。搜索框是音乐客户端的一等公民，这个必须第一天就验证。
5. **建议先把 core（API / 播放 / 队列）做成与 UI 无关的 crate。** 万一 GPUI 顶不住，换 egui / iced / Tauri 只重写 UI 层，不至于推倒重来。

---

## 1. 现有开源客户端调研

### 1.1 桌面端格局

| 客户端 | 技术栈 | 规模 | 许可 | 状态 | 评价 |
|---|---|---|---|---|---|
| **Feishin** | Electron + React + MPV | ~10.4k★ | GPL-3.0 | 活跃 | 功能标杆：Jellyfin/Navidrome/Subsonic 三端统一 controller、智能歌单、歌词、scrobble、gapless。代价是 Electron 的 200–500MB 常驻内存与 80–150MB 安装包 |
| **Sonixd** | Electron + React | — | GPL-3.0 | 停更 | Feishin 的前身，已被取代 |
| **Sonora** | **Rust + GPUI**，rodio/symphonia/cpal，librespot | ~2.4k★ | GPL-3.0 | 活跃（v0.44.2, 2026-10） | **最直接的对标产品**。但它是"多源聚合播放器"（Apple Music/Spotify/YT Music/Deezer/Tidal/Subsonic/本地），不是轻量 Navidrome 客户端。GPUI 用自家 fork 锁 commit，rodio 也 patch 到 RustAudio 的 git rev |
| **Supersonic** | Go + GTK4 + MPV | 成熟 | GPL-3.0 | 活跃 | 架构值得抄：服务端抽象 + 本地缓存 + 播放列表模型。但 GTK/MPV 让 Windows 分发很难看 |
| **Psysonic** | Tauri + Rust + React | ~289★ | — | 活跃 | 轻量路线最接近的对手，但 WebView 渲染 + 前端生态，启动和内存仍不如纯原生 |
| **Rufin** | GTK4 + Rust | ~44★ | — | 活跃 | GNOME 原生，体验好但绑定 Linux |
| **Tributary** | GTK4 + libadwaita + Rust | — | — | 活跃 | 同上，Linux 专用 |
| **Firmium** | Iced + Rust | ~32★ | — | 活跃 | 跨平台含 Android，但 Iced 的即时模式在复杂列表/滚动上吃力 |
| **Nokkvi** | Iced + Rust + 自研 PipeWire 引擎 | ~26★ | — | 活跃 | Linux only，倒是证明了"自研音频引擎"这条路可行 |
| **Aura** | Rust + 原生音频引擎 | — | — | 活跃 | 新项目，细节少 |
| **EKO** | Rust，bit-perfect 输出 | — | — | 活跃 | macOS only，走的是发烧友路线 |
| **NaviBeat / Naviamp** | 原生（未标明框架） | — | — | 活跃 | 有 `--tui` 模式，AppImage/apt/dnf |
| **Strawberry** | Qt/C++ | 大 | GPL | 成熟 | 老牌本地播放器，Subsonic 支持是附加功能 |

终端类：**ferrosonic**、**ratune**（Rust）。
Web 类：tinysub、ampcast、Airdrome、Thunderdrome、Mist、subplayer、airsonic-refix —— 这些说明"服务端能力早就够用了"，桌面端拼的是**体验**而不是协议。

### 1.2 三个关键观察

**观察一：功能最全的（Feishin）最重，最轻的（Rufin/Tributary）最偏科。**
中间地带——"Windows 上开箱即用、原生、轻、够用"——目前只有 Psysonic（Tauri）勉强占着，而它对 WebView 的依赖让它离"原生手感"始终差一口气。

**观察二：Sonora 的存在既是威胁也是利好。**
威胁：它已经用 Rust + GPUI 证明了技术路线可行，而且拿了 2.4k star。
利好：它的野心是"一个客户端接所有音乐服务"，这个定位必然导致：
- 依赖树巨大（librespot、ytmusic、webview、widevine、OAuth……）
- 代码库对 GPUI 的 fork 深度绑定
- 不可能为单一 Navidrome 场景做极致优化

**你的机会在"减法"**：单一协议、单一服务端、无 DRM、无第三方 SDK，二进制可以做到 Sonora 的三分之一。

**观察三：中文场景基本没人做。**
- 中文搜索 / 拼音输入（IME）在 GPUI 上是已知雷区
- 中文标签、繁简、CJK 字体回退、歌词时间轴
- Navidrome 中文社区用户普遍自建、在意资源占用

这是一个被忽视的细分市场。

### 1.3 功能基线（用户预期）

从上述客户端的交集看，2026 年一个 Navidrome 桌面客户端的"及格线"是：

- 连接 + 凭据保存（token 认证）
- 浏览：艺人 / 专辑 / 歌单 / 收藏 / 最近添加 / 随机
- 搜索（艺人/专辑/歌曲分栏）
- 播放队列、拖拽排序、随机、循环、上一首/下一首
- **无缝播放（gapless）**
- Seek、音量、静音
- 封面图缓存
- 滚动歌词 / 逐行歌词
- Scrobble（Last.fm / ListenBrainz）
- 系统媒体键 / SMTC / MPRIS

"加分项"：离线下载、智能歌单、EQ、交叉淡入、ReplayGain、多服务器、UPnP。

---

## 2. 产品定位建议

### 一句话定位

> **一个 25MB 以内、冷启动 1 秒、空闲 80MB 内存的原生 Navidrome 客户端，Windows 优先，无缝播放，中文开箱即用。**

### 明确的不做清单（Non-goals）

这些是 Sonora / Feishin 的地盘，碰了就会输：

- ❌ Spotify / Apple Music / YouTube 等第三方音乐源（DRM、合规、依赖爆炸）
- ❌ Jellyfin / Emby 多后端适配（v1 不做，接口留好）
- ❌ 视频、播客、有声书
- ❌ 内置转码 / 服务端管理
- ❌ 离线曲库同步（v1 不做，v2 再做）

### 差异化抓手

1. **启动与内存**：这是唯一能直观打败 Electron 和 Tauri 的指标
2. **gapless 的正确性**：很多客户端号称支持，实际在边界处仍有 10–30ms 空隙
3. **中文体验**：IME、字体、歌词、排序
4. **单文件分发**：Windows 绿色版 exe，无运行时依赖

---

## 3. 技术选型 A：GPUI 现状

### 3.1 版本与分发现状（截至 2026-11）

| 渠道 | 版本 | 说明 |
|---|---|---|
| crates.io 官方 `gpui` | **0.2.2**（2025-10-22） | **陈旧**，落后 Zed 主线约 300 天 / 500+ commit |
| `gpui-unofficial` | v1.25.2 | 社区镜像，每 6 小时同步 Zed tag 到 crates.io |
| `gpui-ce`（Community Edition） | 0.3.0（MSRV 1.95） | 社区共建 fork，`package = "gpui-ce"` 别名成 `gpui`，面向第三方应用 |
| `gpui-pre` | 0.3.12（周更） | Longbridge 的周快照，配 `gpui-kit` / `gpui-component` 门面 |
| Zed 仓库 git 依赖 | 最新 | 官方推荐方式，但 commit 漂移、重复编译 |

官方 gpui.rs 的原话：**"For the near future, GPUI is tied to Zed."** Zed 曾计划 2026 年 9 月底做自动发布，但没兑现。

### 3.2 平台支持

- **macOS**：Metal（shader 需要 Xcode 的 Metal 编译器，CI 上要装）
- **Windows**：DirectX（`gpui_windows`）
- **Linux**：X11 + Wayland，**wgpu/Vulkan**（Zed 在 2026-02 的 PR #46758 从 Blade 切到 wgpu，这个改动不在 0.2.2 里）

### 3.3 已知坑

1. **依赖地狱**：两个库各自 pin 不同 GPUI commit → Cargo 编译两份 → `expected WindowContext, found WindowContext`。解法只有 `[patch.crates-io]` 或 vendor。
2. **没有官方组件库**：GPUI 只给你布局、绘制、文本、事件。按钮、输入框、滚动条、模态框全部自建（或用 `gpui-component`、`gpui-base` 等社区库，质量参差）。
3. **IME**：Zed issue #56327 记录了 Windows 上中文输入法导致的重复插入与 DirectWrite 崩溃。根因是 `WM_KEYDOWN → ToUnicode` 和 `WM_CHAR` 两条路径都往输入框写字符，且没有检查 IME composition 状态。社区 `tauri-plugin-gpui` 也注明 Windows IME 路径"仅编译检查，未验证"。
4. **API 不稳定**：0.x 每个版本都可能破坏性变更，0.2.2 → 1.25 之间 API 变化很大。

### 3.4 三条可选技术栈

实测拉取到的版本（2026-11-11）：

| 栈 | 组成 | 版本关系 | 优点 | 风险 |
|---|---|---|---|---|
| **A. 上游镜像** | `gpui-unofficial` 1.28.0-pre | 跟随 Zed tag | 最接近上游，问题可对照 Zed issue | 无组件库，UI 全手写；版本带 `-pre` |
| **B. 社区版** | `gpui-ce` 0.3.0 | MSRV 1.95 | 面向第三方应用，有治理 | 版本落后上游较多，生态小 |
| **C. 组件生态** | `gpui-pre` 0.3.12 + `gpui-component` 0.9.1 | 精确锁定 `=0.3.12` | **60+ 组件**（Input/List/Button/Dialog），省掉大量 UI 工作量 | 绑定 Longbridge 的周快照；依赖体积需实测 |

**推荐：Phase 0 同时验证 A 与 C，重点看中文输入法。**

理由：
- C 的 `Input` 组件如果已经正确处理 IME，R1（输入法致命风险）和 R5（无组件库）会同时消掉，性价比最高；
- A 是"控制组"：如果 C 的 Input 有问题，至少知道是组件层还是 GPUI 底层的问题；
- 两条栈的 API 都基于同一套 GPUI 概念，切换成本主要在视图层，不会影响 `melos-core`。

**工具链要求**：`gpui-ce` 声明 MSRV 1.95，本机默认 `stable` 是 1.92，**必须用已安装的 1.99.0**（`cargo +1.99.0-x86_64-pc-windows-msvc`）。

**必须做的隔离**：所有 GPUI 相关代码只出现在 `melos-app` 一个 crate 里；`melos-api` / `melos-player` / `melos-core` 不得 `use gpui::*`。

**备选方案**（如果两条 GPUI 栈都顶不住）：
- **egui**：生态最成熟、IME 最稳、代码量最小，缺点是默认视觉"工具感"强，需要自己打磨
- **iced**：Elm 架构、好看，但复杂列表性能一般（Firmium/Nokkvi 的经验）
- **Tauri**：最保险，但直接落入 Psysonic 的定位，失去"纯原生"卖点

> **Phase 0 实测结论（2026-11-11）**：A 与 C 的中文 IME 均通过；A 的 release 为 10.1MB / 54MB 工作集，C 的 debug 为 51.1MB，按阈值规则选择 **A（gpui-unofficial）**。详见 [ADR-001](adr/0001-ui-stack.md)。

---

## 4. 技术选型 B：音频引擎

### 4.1 候选对比

| 方案 | 优点 | 缺点 | 结论 |
|---|---|---|---|
| **rodio 0.22** | 上手快，`Decoder::builder().with_gapless(true).with_seekable(true)`，底层就是 cpal | 2026 年 API 大改（`OutputStream→DeviceSink`、`Sink→Player`），gapless 仍偏实验性，多格式支持依赖 feature | **MVP 首选** |
| **symphonia 0.5.5 + cpal 0.18 自建** | 完全可控：gapless/seek/ReplayGain/淡入淡出都自己说了算；单文件；性能最好 | 工作量大，需要处理 resample（rubato/ardftsrc）、无锁环形缓冲（rtrb）、实时回调安全 | **v1 目标形态** |
| **libmpv** | 格式兼容性无敌，gapless 成熟 | 外部依赖、LGPL/GPL、Windows 打包 30MB+，直接毁掉轻量定位 | ❌ |
| **GStreamer** | 全 | 极重，插件地狱 | ❌ |

### 4.2 推荐引擎结构

```
HTTP (reqwest, Range)
  → 环形缓冲 / 磁盘预取
  → symphonia Demuxer + Decoder
  → 重采样 (rubato or ardftsrc) → 统一 f32 / 48kHz
  → rtrb SPSC 无锁队列
  → cpal 实时回调（只做 memcpy + 混音 + 音量）
```

关键点：
- **解码线程与音频回调严格分离**，回调里禁止分配、加锁、日志
- **gapless**：把队列里下一首的已解码帧直接接在 ring buffer 后面，不 flush 设备
- **seek**：优先 `format=raw` + HTTP Range；服务端转码流不支持 Range 时，回退到重新请求并把偏移量告知服务端（OpenSubsonic 的 `transcodeOffset` 扩展，需运行时探测）
- **格式回退**：symphonia 覆盖 mp3/flac/ogg-vorbis/aac/mp4 足够；Opus 视情况加 `opus`/`ogg` crate；实在不支持就让服务端 `maxBitRate` 转 mp3

### 4.3 与 Sonora 的差异

Sonora 用 `rodio 0.22` 但 **patch 到了 RustAudio 的 git rev**，还额外引了 `rusty_aac`、`rustfft`、`ardftsrc` 和自己的重采样路径。这说明 rodio 的默认能力不够用。

**建议**：MVP 用原版 rodio 快速验证；一旦遇到 gapless 缝隙或 seek 精度问题，按 Sonora 的路线升级为自建引擎，而不是跟着 patch 别人的分支。

### 4.4 不用 MPV 的代价，以及怎么补

MPV 在这类客户端里提供的价值其实只有四项：**格式覆盖、gapless 成熟度、音频设备适配、网络容错**。逐项拆。

#### 4.4.1 核心思路：把"兼容性"外包给 Navidrome

我们不是通用播放器，是 Navidrome 客户端。Subsonic 的 `Song` 对象自带 `suffix` 和 `contentType`，**播放前就能知道编码**：

- 本地解码器支持 → `stream?format=raw`：原始字节，支持 HTTP Range（精确 seek），无转码延迟，gapless 最好
- 不支持 → `stream?format=mp3&maxBitRate=xxx`：由 Navidrome 端的 ffmpeg 转码

**重要修正（Phase 0 实测）**：symphonia 0.6.1 没有随包提供 Opus 解码器，而 Navidrome 的 `DefaultDownsamplingFormat` 默认是 **opus**。因此请求转码时必须**显式写 `format=mp3`**，否则可能拿到无法解码的 Opus 流。MP3 有 encoder delay/padding，转码流在曲目边界可能有几毫秒空隙，但只影响冷门格式；本地可解码的格式走 `format=raw`，不受影响。

这样，"客户端什么都能解"这个 MPV 最大的卖点，在自建服务场景里被服务端覆盖了。

#### 4.4.2 本地解码矩阵（symphonia 0.5.5）

| 格式 | 本地解码 | 回退 |
|---|---|---|
| mp3 / flac / ogg-vorbis / wav / aiff | ✅ | — |
| opus | ❌（0.6.1 未随包发布解码器） | mp3 转码 |
| alac | ✅ | — |
| wavpack | ❌（第三方 crate 为 GPL-3.0，未集成） | mp3 转码 |
| m4a / aac-lc | ✅ | HE-AAC（SBR）可能失败 → mp3 转码 |
| wma / ape / dsd（dsf, dff） | ❌ | mp3 转码 |
| >2 声道 | ❌ | 降混立体声或转码 |

覆盖自建音乐库 95%+ 的实际内容。剩下的冷门格式，转码成本由服务端承担，且只影响个别文件。

#### 4.4.3 逐项替代方案

| MPV 能力 | 我们的替代 | 代价 |
|---|---|---|
| WASAPI 独占 / bit-perfect | **明确 non-goal**，共享模式输出 | 放弃发烧友场景 |
| 采样率自动切换 | 设备默认采样率优先；源采样率一致则不重采样，不一致用 rubato | 无（MPV 也常重采样） |
| 多声道 / 环绕 | 只支持立体声 | 极少场景 |
| 损坏文件鲁棒性 | 解码在独立线程 + `catch_unwind`；失败自动降级为服务端转码 | 多一层兜底 |
| 网络抖动 | 2–4 秒预缓冲、双缓冲、Range 指数退避重试、断流从 position 续 | 自己做，约 3–5 天 |
| EQ / crossfade / ReplayGain | biquad 自研（~200 行）、淡入淡出、`ebur128` | 排到 v2 |
| 格式探测 | `suffix` + `contentType` + symphonia `probe` 双重判断 | 无 |

#### 4.4.4 保留一个逃生通道

如果半年后确实出现格式投诉集中，加一个 **feature-gated 的 libmpv 后端**：默认构建不含，单独发 "full" 版安装包。架构上把 `PlaybackBackend` 做成 trait（`play / pause / seek / position / set_queue`），libmpv 只是第二个实现。

**既守住默认版的轻量定位，又不把技术路线钉死。**

#### 4.4.5 量化对比

| | 自建 symphonia + cpal | 捆绑 libmpv |
|---|---|---|
| 二进制增量 | ~3–5MB | ~35–45MB |
| 进程模型 | 单进程 | 子进程 + IPC 或动态库 |
| gapless 可控性 | 完全可控 | 依赖 mpv 配置 |
| 设备独占 | 不支持 | 支持 |
| 冷启动 | 无额外初始化 | mpv 初始化 100–300ms |
| 开发成本 | +1–2 周 | 集成 2–3 天 |

---

## 5. 技术选型 C：服务端接口

### 5.1 只做 Subsonic / OpenSubsonic，不做 Navidrome 私有 API

Navidrome 有一套自己的 REST API（`/auth/login`、`/api/album` 等），Feishin 用它来获得更好的分页和搜索。但：
- 无正式文档、随版本漂移
- 换服务端就废

**决定：v1 只用 Subsonic API + OpenSubsonic 扩展探测**，保证对 Navidrome / Airsonic / Gonic / LMS 的兼容。

### 5.2 认证

```
u = username
c = Melos
v = 1.16.1
s = 每次请求随机 salt（≥6 字符）
t = md5(password + salt) 的十六进制小写
f = json
```

- 密码只在首次登录时使用，**磁盘上不保存明文**：保存 `md5(password + 固定salt)` 不行（每请求 salt 不同），因此要么保存密码（加密存储），要么用 OpenSubsonic 的 **apiKey** 认证。
- **建议**：优先探测 `apiKeyAuthentication` 扩展；没有则用 OS 凭据库（Windows Credential Manager / macOS Keychain / Secret Service）保存密码，内存中算 token。

### 5.3 MVP 需要的端点

| 用途 | 端点 |
|---|---|
| 连通性 | `ping` |
| 浏览 | `getArtists` / `getArtist` / `getAlbum` / `getAlbumList2` |
| 详情 | `getSong` |
| 搜索 | `search3` |
| 播放 | `stream`（`format=raw`, `maxBitRate`, `estimateContentLength`）、`download` |
| 封面 | `getCoverArt`（`size` 参数） |
| 统计 | `scrobble`、`star` / `unstar`、`getStarred2` |
| 歌单 | `getPlaylists` / `getPlaylist` |
| 扩展探测 | `getOpenSubsonicExtensions` |
| 歌词 | `getLyricsBySongId`（OpenSubsonic `songLyrics` 扩展） |

错误码按 `10/20/30/40/41/50/60/70` 映射成用户可读提示（尤其 40 = 密码错、70 = 找不到）。

### 5.4 缓存策略

- 封面：磁盘 LRU（默认 256MB），key = `coverArtId + size`
- 元数据：内存 + 轻量持久化（`rusqlite` 或 `sled`，建议 SQLite，便于调试）
- 列表分页：`getAlbumList2` 的 `size` 上限 500，用 offset 翻页
- 请求合并：同一封面/元数据并发请求去重（`DashMap<T, OnceCell>`）

---

## 6. 架构设计

### 6.1 Workspace 划分

```
Melos/
├─ crates/
│  ├─ melos-api/       # Subsonic 客户端：认证、端点、DTO、错误、重试
│  ├─ melos-player/    # 解码 → 重采样 → ring buffer → cpal；队列不在这里
│  ├─ melos-core/      # 领域模型、队列状态机、缓存、设置、scrobble（无 UI 依赖）
│  ├─ melos-app/       # GPUI 界面、Actions、平台集成、入口（唯一依赖 gpui）
│  └─ melos-tui/       # ratatui 终端界面（依赖 melos-core，不依赖 gpui）
├─ assets/          # 图标、字体（含 CJK 子集）
├─ packaging/       # Windows/macOS/Linux 打包脚本
├─ spikes/          # 独立于 workspace 的技术验证工程
└─ docs/
```

依赖方向严格单向：`melos-app → melos-core → melos-player → melos-api`，`melos-tui → melos-core`。`melos-core` 不得 `use gpui::*` 或 `use ratatui::*`——两条前端线互不知情。

### 6.2 状态模型

- `AppState`：登录态、当前视图、设置
- `LibraryState`：艺人/专辑/歌单缓存
- `QueueState`：当前队列、索引、随机序列、循环模式
- `PlayerState`：播放/暂停、位置、时长、音量、缓冲

GPUI 中用 `Entity<T>` + `cx.observe` / `cx.subscribe` 驱动重绘；播放器线程通过 channel 发事件，UI 侧在 `cx.spawn` 里消费并更新 Entity。

### 6.3 关键路径

**播放一首歌**

```
UI click → melos-core: queue.set_playlist(album, index)
        → melos-player: play(track)
        → melos-api: stream_url(id, raw, bitrate)
        → reqwest Range 请求 → decoder
        → cpal 输出
        → 每 500ms 上报位置；到 50% 发 scrobble(submission=false)，到 90% 发 submission=true
```

**切歌（gapless）**

```
位置 >= duration - preload_threshold (5s)
  → 预取下一首前 N 秒解码进 ring buffer
  → 当前曲目最后一帧输出后直接续播，不 flush
```

### 6.4 UI 结构

```
┌──────────┬──────────────────────────────────┐
│ 侧边栏    │  内容区（虚拟化列表 / 网格）        │
│ 首页      │                                  │
│ 艺人      │                                  │
│ 专辑      │                                  │
│ 歌单      │                                  │
│ 收藏      │                                  │
│ 搜索      │                                  │
│ 设置      │                                  │
├──────────┴──────────────────────────────────┤
│ 正在播放条：封面 标题/艺人  ◀ ▶  ⏯  进度  音量 │
└─────────────────────────────────────────────┘
```

- 列表用 GPUI 的 `uniform_list` / 自定义虚拟化，避免一次性构建上千个 element
- 封面异步加载 + 占位色块，滚动时不阻塞
- 主题：一套 token（颜色/圆角/间距）集中在 `theme.rs`，先做深色，浅色后补

### 6.5 性能预算

| 指标 | 目标 |
|---|---|
| 冷启动到可交互 | < 1.0s |
| 空闲内存 | < 80MB |
| 播放时内存 | < 120MB |
| 二进制（strip 后） | < 25MB |
| 空闲 CPU | < 1% |
| 列表滚动 | 60fps @ 10k 行 |
| 封面加载 | 首屏 < 300ms |

### 6.6 沉浸式视觉设计

**核心原则：氛围来自封面，而不是来自特效。** 沉浸式最容易做砸的地方，是用实时模糊和持续动画堆质感，最后把"轻量"两个字吃掉。所以：

**背景**
- 取当前曲目封面，降采样到 16×16，再用线性插值放大铺满窗口 → 视觉上等价于柔和模糊，成本接近零
- 叠一层 60–80% 的深色遮罩保证文字对比度
- 只在**封面变化**或**窗口尺寸变化**时重算，结果缓存成纹理；滚动、拖动进度条不触发重算
- 不使用实时 backdrop blur（Windows 上尤其贵，且 GPUI 跨平台支持不一致）

**布局**
- 左侧栏可收起：展开 240px，收起 64px 图标模式
- 主区域：内容列表 + 右侧可展开的播放队列抽屉
- 正在播放全屏视图：封面占 40–45% 宽，右侧信息 + 歌词（v1.1）
- 底部播放条常驻：56px 高，封面 40×40，进度条为细线，hover 才显示时间

**色彩与排版**
- 从封面提取主色（降采样后取加权平均，做饱和度/明度归一化）作为 accent，用于进度条、选中态、hover
- 深色为默认：背景 `#0B0B0E`，面板 `#141418`，文字三档（主/次/弱）
- 字体：Inter + 思源黑体子集；数字用 tabular figures，避免进度时间跳动
- 圆角 12/16px，8pt 间距网格

**动效**
- 只做两类：播放/暂停的 120ms 缩放反馈、封面切换的 200ms 交叉淡入
- **禁止**持续运行的动画（可视化、呼吸灯、无限旋转），空闲时 GPU/CPU 必须归零
- 设置里提供"减少动效"开关，尊重系统无障碍设置

**与性能预算的冲突检查**
- 背景降采样 + 放大：一次计算 < 1ms，纹理常驻 < 2MB
- 封面按显示尺寸请求（`getCoverArt?size=300`），避免把 1200px 原图拉进来
- 列表虚拟化 + 图片异步加载，滚动时不出图也不阻塞

### 6.7 TUI 前端

TUI 不是"顺便加的"，它是**架构正确性的测试**：如果 `melos-core` 能被终端完整驱动，说明业务逻辑和 UI 真的解耦了。

**技术栈**：`ratatui` + `crossterm`

**布局**

```
┌─ 库 ─────────┬─ 曲目 ──────────────────────┬─ 队列 ──────┐
│ ▸ 专辑        │  01  Title           3:42   │  Now Playing │
│ ▸ 艺人        │  02  Title           4:01   │  ───────────  │
│ ▸ 歌单        │  03  Title           2:58   │  Next up...  │
│ ▸ 收藏        │                             │              │
├──────────────┴─────────────────────────────┴──────────────┤
│ ▶ Artist — Album · 01:23 / 03:42   vol 80%   flac 44.1k   │
└───────────────────────────────────────────────────────────┘
```

**交互**
- vim 风格：`j/k` 移动、`h/l` 切栏、`Enter` 播放、`Space` 暂停、`/` 搜索、`q` 退出
- 搜索框需要中文输入——终端本身由终端模拟器处理 IME，**这正是 TUI 相对 GPUI 的优势**：IME 风险为零
- 封面：可选。默认用 ANSI 色块占位；支持 sixel/kitty 协议的终端可显示缩略图（`ratatui-image`）
- 播放器与 GUI 共用 `melos-player`，后台线程不变

**分发**
- 同一个 `melos` 二进制，`melos --tui` 进入终端模式
- 无 GUI 依赖时（Linux 服务器）可编译 `--no-default-features` 的纯 TUI 版本，体积 < 6MB

**成本**：3–5 天（复用 core，只写视图层）。

---

## 7. MVP 范围与里程碑

### Phase 0：技术验证（2–3 天，先做，做完再决定要不要继续）

这是整个计划里最关键的一步，**必须先证伪**：

1. GPUI 在新机器上跑起 Hello World（Windows）
2. **中文输入法在 GPUI 输入框里能否正常工作**（搜狗/微软拼音各测一遍）
3. symphonia + cpal 播放一个远程 URL（Navidrome `stream?format=raw`）
4. gapless：连续播两首，用波形/听觉确认无缝隙
5. 打包一个 20MB 级别的 exe，冷启动计时

> 如果 2 或 4 不达标，立刻切换方案（egui / Tauri），不要在 GPUI 上硬扛。

### Phase 1：v1.0 首发（3–4 周）

- 登录/连接、凭据保存（OS 凭据库）、错误提示
- 侧边栏 + 专辑网格 + 专辑详情 + 艺人页
- 播放队列、播放/暂停/上下曲/seek/音量、随机/循环
- **gapless + 格式回退（raw / opus 自动切换）**
- 搜索（3 类）
- 收藏 / star
- Scrobble（Last.fm / ListenBrainz）——成本极低，自建用户在意
- 封面缓存
- 系统媒体控制（Windows SMTC）
- 深色主题、播放位置持久化

### Phase 2：v1.1（2–3 周）

- 歌词（OpenSubsonic `getLyricsBySongId`，回退 `getLyrics`）
- 歌单浏览与播放
- 设置页（转码偏好、缓存目录、语言）
- 浅色主题
- **TUI 前端（beta）**：`--tui` 启动，复用 `melos-core`

### Phase 3：分发（3–5 天）

- Windows：`cargo build --release` + 单文件 exe；可选 NSIS 安装包；签名（否则 SmartScreen 警告）
- macOS：`.app` 打包 + ad-hoc 签名；Metal shader 编译进 CI
- Linux：AppImage / Flatpak
- 自动更新：可选（`self_update` 或 GitHub Releases 检查）

**合计：约 5–7 周（业余时间按 ×1.8 估）**

---

## 8. 风险登记表

| # | 风险 | 概率 | 影响 | 应对 |
|---|---|---|---|---|
| R1 | GPUI Windows IME 不可用 | 中高 | **致命** | Phase 0 首日验证；不行就 fork 修（参考 Zed #56327）或换 egui |
| R2 | GPUI API 漂移导致升级困难 | 高 | 中 | 锁版本 + vendor；不追新 |
| R3 | gapless 有可闻缝隙 | 中 | 高 | 预留自建引擎方案；先做双曲接缝测试 |
| R4 | 转码流无法 seek | 中 | 中 | 优先 raw；探测 `transcodeOffset`；降级为重新请求 |
| R5 | 没有组件库，UI 工作量翻倍 | 高 | 中 | 先用最小控件集；评估 `gpui-component` |
| R6 | 范围蔓延（对标 Feishin） | 高 | 高 | 严守 Non-goals；每加一功能问"这会让它变重吗" |
| R7 | 构建时间过长（GPUI 依赖重） | 中 | 低 | `sccache`、增量构建、CI 缓存 |
| R8 | 许可问题 | 低 | 中 | GPL-3.0 已定；GPUI 为 Apache-2.0、symphonia 为 MPL-2.0，均兼容；发行前过 `cargo deny` |
| R9 | 冷门格式本地解码失败（WMA/APE/DSD） | 中 | 低 | 自动 opus 转码；UI 标注"转码播放"；预留 feature-gated libmpv 后端作逃生通道 |
| R10 | 沉浸式视觉拖累性能（模糊/动画） | 中 | 中 | 禁止实时模糊与持续动画；背景用 16×16 降采样放大；纹理缓存；提供"减少动效"开关 |
| R11 | 双前端导致 core 层泄漏 UI 依赖 | 中 | 中 | CI 加约束：`melos-core` 的 Cargo.toml 不允许出现 gpui/ratatui；PR 检查 |

---

## 9. 决策记录

### 已锁定（2026-11-11）

| 项 | 决定 |
|---|---|
| 首发平台 | **Windows 优先**（macOS / Linux 架构兼容，但不作为首发验收标准） |
| v1 功能底线 | Phase 1 清单：播放闭环 + 浏览 + 搜索 + 队列 + star + scrobble + SMTC，**不含歌词** |
| 外部依赖 | **不引入 WebView / MPV / GStreamer**；格式靠 symphonia + 服务端转码 |
| 许可证 | **GPL-3.0** |
| 视觉风格 | **沉浸式**：大封面 + 封面衍生背景 + 低密度信息层级 |
| 形态 | **GUI + TUI 双前端**，共用 `melos-core`；TUI 用 ratatui |
| 核心定位 | **轻量**：25MB 二进制、1 秒冷启动、80MB 空闲内存。任何功能不得突破性能预算 |

许可证兼容性说明：GPUI 为 Apache-2.0，symphonia 为 MPL-2.0，均与 GPL-3.0 兼容（发行时保留第三方声明，`cargo deny` 校验）。

### 仍待定

1. **macOS / Linux 首发配套**：打包脚本可以先写好，不阻塞 Windows 验收。
2. **TUI 是否随 v1.1 一起发**（建议是，成本约 3–5 天，可直接复用 core）。

---

## 10. 参考来源

- Navidrome 官方与客户端列表：https://www.navidrome.org/ ，https://navidrome.org/apps/
- Sonora（Rust + GPUI 对标项目）：https://github.com/sonorahq/sonora
- Feishin：https://github.com/jeffvli/feishin ；Navidrome 集成分析：https://deepwiki.com/jeffvli/feishin/3.3-navidrome-and-subsonic-integration
- GPUI 官网：https://gpui.rs/
- GPUI 分发问题：https://gpui-archipelago.github.io/news/gpui-distribution
- GPUI 社区版：https://lib.rs/crates/gpui-ce
- Zed IME issue：https://github.com/zed-industries/zed/issues/56327
- Subsonic API：https://www.subsonic.org/pages/api.jsp
- OpenSubsonic 扩展：https://opensubsonic.netlify.app/docs/
- rodio 解码器文档：https://docs.rs/rodio/latest/rodio/decoder/index.html
- audio-stack-rs（cpal + symphonia 参考实现）：https://lib.rs/crates/audio-stack-rs
