# Melos 实施计划

> 依据：[调研与技术方案](00-research.md)
> 日期：2026-11-11　更新：2026-11-12　状态：v1.0 功能收尾与验收中
> 估算口径：1 工程日 = 6 小时，单人

---

## 0. 使用方式

- **门禁制**：每个阶段有明确的通过标准，不达标不进入下一阶段。
- **先证伪**：Phase 0 只验证假设，不写产品代码；宁可三天推翻，不要三周返工。
- **估算含缓冲**：表中数字已按业余时间折算，不含突发故障。

---

## 1. 总览

| 阶段 | 内容 | 工期 | 出口标准 |
|---|---|---|---|
| **Phase 0** | 技术验证（GPUI A/C、IME、音频、性能） | 2–3 天 | ADR-001 选定 UI 栈；音频路径可行 |
| **Phase 1** | v1.0：播放闭环 | 3–4 周 | 能替代 Feishin 日常听歌 |
| **Phase 2** | v1.1：歌词、歌单、TUI | 2–3 周 | TUI 可用；歌词同步准确 |
| **Phase 3** | 分发 | 3–5 天 | Windows 安装包 + Linux AppImage |

---

## 2. Phase 0：技术验证（2–3 天）

### 2.1 要回答的六个问题

| # | 问题 | 对应风险 |
|---|---|---|
| Q1 | GPUI 能否在 Windows 11 稳定开窗？ | R2 |
| Q2 | 中文输入法在 GPUI 输入框里是否可用？ | **R1（致命）** |
| Q3 | 远程音频流能否稳定播放？ | — |
| Q4 | 双曲接缝是否真的无缝？ | R3 |
| Q5 | 体积 / 启动 / 内存基线是多少？ | 定位 |
| Q6 | 依赖能否锁定并复现构建？ | R2 |

### 2.2 门禁标准

**G1 窗口**
- Windows 11 打开 900×600 窗口，显示 10 分钟无崩溃
- 空转 CPU ≈ 0，拖动/滚动无撕裂
- release 构建可运行

**G2 中文输入法（最重要）**

微软拼音、搜狗输入法各测一遍，全部满足才算通过：

| 用例 | 通过标准 |
|---|---|
| 输入「你好世界」 | 逐字正确，无重复、无丢字 |
| 输入「测试，标点。」 | 中文标点正确 |
| 输入「nǐ hǎo」 | 拼音组合与候选选择正常 |
| 组合中按退格 | 只删组合串，不误删已上屏文字 |
| 组合中按 Esc | 取消组合，不残留字符 |
| 候选框位置 | 跟随光标，偏差 < 20px |
| 连续输入 50 字 | 无重复、无卡顿 |
| 失焦再聚焦 | 输入正常，光标位置正确 |
| 崩溃 | 0 次 |

**G3 远程播放**
- 从真实 Navidrome 拉 `stream?format=raw`，播放 60 秒
- 无爆音、无卡顿、无断流
- 单核 CPU < 10%

**G4 Gapless**
- 两首 FLAC 背靠背播放
- 输出回调写盘分析：接缝间隙 < 5ms
- 无可闻 click

**G5 性能基线（记录，不要求达标）**

| 指标 | Phase 0 允许值 | v1.0 目标 |
|---|---|---|
| exe（release + strip） | < 35MB | < 25MB |
| 冷启动到窗口可见 | < 1.2s | < 1.0s |
| 空闲内存 | < 120MB | < 80MB |
| 空转 CPU | ≈ 0 | ≈ 0 |

**G6 构建可复现**
- `cargo build --release --locked` 成功
- 记录：依赖总数、首次构建时长、target 体积
- `cargo vendor` 可执行

### 2.3 Day 1 上午：环境 + Spike A（gpui-unofficial）

**环境（30 分钟）**

```powershell
# 全部命令经 vcvars 启动，避免 Git Bash 的 link.exe 抢占
cmd /c '"C:\Program Files (x86)\Microsoft Visual Studio\18\BuildTools\VC\Auxiliary\Build\vcvars64.bat" && cargo +1.99.0-x86_64-pc-windows-msvc --version'
```

记录到 `docs/02-phase0-report.md`：工具链、构建时长、依赖数量。

**Spike A：`spikes/gpui-a/`**

```toml
[package]
name = "spike-gpui-a"
version = "0.1.0"
edition = "2024"

[dependencies]
gpui = { package = "gpui-unofficial", version = "=1.28.0-pre" }
```

任务：
- A1 打开窗口，渲染标题与一块背景色（0.5h）
- A2 实现最小输入框：`EntityInputHandler` + `window.handle_input`（**最大不确定项，时间盒 4h**）
- A3 记录 IME 行为（交给用户手测）
- A4 记录二进制体积、启动时间、内存

> 说明：`gpui-unofficial` 的所有内部 crate 都带 `-gpui-unofficial` 后缀重新发布，不会与官方 `gpui` 冲突。这是它相对 git 依赖的最大优势。

### 2.4 Day 1 下午：Spike C（gpui-pre + gpui-component）

**`spikes/gpui-c/`**

```toml
[dependencies]
gpui = { package = "gpui-pre", version = "=0.3.12" }
gpui-component = "=0.9.1"
```

任务：
- C1 打开窗口 + 初始化组件库（0.5h）
- C2 放一个单行 `Input`（0.5h）
- C3 IME 手测
- C4 与 A 对比：体积、启动、依赖数

> `gpui-component` 精确依赖 `gpui-pre = 0.3.12`，两者是同一生态，不存在版本漂移。

### 2.5 Day 2：音频冒烟（`spikes/audio/`）

依赖（实测版本）：

```toml
symphonia = { version = "0.6.1", features = ["all"] }
cpal = "0.18.2"
rubato = "5.0.1"
rtrb = "0.4.1"
```

> 注意：symphonia 0.6.1 的 `all` 不含 Opus 解码，而 Navidrome 默认转码格式是 Opus。M1.4 的流 URL 必须显式带 `format=mp3`，不能只传 `maxBitRate`。

任务：
- D1 本地文件解码：flac / mp3 / aac(m4a) / opus 各 30 秒（2h）
- D2 cpal 输出，确认设备枚举与默认设备（1h）
- D3 双曲背靠背：预加载第二首，接缝不 flush（1.5h）
- D4 输出回调写盘 → 分析间隙（1h）
- D5 seek 精度测试（0.5h）

**测试素材**：优先用你自己的 Navidrome 曲库（`stream?format=raw` 另存）；没有的话用 public domain 样例，覆盖四种格式。

**失败处理**：
- 解码 panic → 记录文件与调用栈，该格式标记为「服务端转码」
- 接缝 > 5ms → 允许 1 天排查；仍失败则先接受 20ms，记入已知问题

### 2.6 Day 3：IME 手测 + 性能 + 报告

- 用户按 G2 表格逐项测试 A/C 两个窗口
- 用秒表/录屏测冷启动；任务管理器记内存
- 写 `docs/02-phase0-report.md`
- 写 `docs/adr/0001-ui-stack.md`

### 2.7 决策规则

```
A 与 C 的 IME 都通过
  → 选 C（组件库省 1 周工作量）
  → 除非：C 的 exe 比 A 大 8MB 以上，或空闲内存多 30MB 以上

只有 A 通过
  → 选 A，Phase 1 增加 1 周自建控件预算（Input / Button / List / Dialog）

只有 C 通过
  → 选 C（更省事），记录 A 的失败原因

两者都不通过
  → egui 做 2 天同规格验证
  → 通过则换 UI 栈；melos-core / melos-api / melos-player 不受影响

音频不达标
  → 先排查 1 天；仍不达标则接受并记录，进入 Phase 1 继续优化
```

### 2.8 产出物

- `spikes/gpui-a/`、`spikes/gpui-c/`、`spikes/audio/`
- `docs/02-phase0-report.md`（数据 + 结论）
- `docs/adr/0001-ui-stack.md`（选定栈与理由）
- 更新 `docs/00-research.md` 的版本与风险表

---

## 3. Phase 1：v1.0（3–4 周）

### 3.1 任务分解

#### M1 `melos-api`（3.5 天）

> **状态：API 能力已实现**。`Client::new/ping`、浏览、搜索、歌单、流 URL 能力矩阵、错误映射、超时与重试均已实现。当前 API 16 个测试通过；桌面歌单 UI 与内存/磁盘封面缓存也已接入。

| ID | 任务 | 产出 | 天 |
|---|---|---|---|
| M1.1 | 请求构造 + token 认证 + `ping` | `Client::new/ping`，集成测试 | 0.5 |
| M1.2 | 浏览端点 | `getArtists/getArtist/getAlbum/getAlbumList2` | 1.0 |
| M1.3 | 搜索与歌单 | `search3`、`getPlaylists/getPlaylist` | 0.5 |
| M1.4 | 流 URL 与能力矩阵 | `stream_url(song)` 依据 `suffix/contentType` 选 raw/mp3；转码显式 `format=mp3` | 0.5 |
| M1.5 | 封面缓存 | 磁盘 LRU 256MB + 并发去重 | 0.5 |
| M1.6 | 错误映射 + 重试 | 10/40/50/70 用户可读；指数退避 | 0.5 |

验收：mock 服务端跑通全部端点；错误码覆盖测试。

#### M2 `melos-player`（4.5 天）

> **状态：核心能力已实现（2026-11-12）**。文件与 HTTP（Range）源、symphonia 解码、声道转换、重采样、cpal 输出、gapless 预取、seek、音量、位置上报已实现。本次补齐整个待播列表预取、队列修改时的未来音频替换、取消旧流恢复与切歌后的 seek；验证结果见 [验收记录](03-acceptance.md)。
>
> **两处与计划的偏差（已记录为后续优化项）**：
> 1. 0.1.4 将 `Mutex<VecDeque<f32>>` 替换为自有固定容量原子 PCM 环形队列，而不是 `rtrb`。常规写入仅序列化生产者，回调不取生产者锁；seek/队列编辑通过独立门控与回调互斥，同时保护播放帧计数。忙于编辑时回调立即静音；解码线程持锁不再影响已有 PCM 的读取。没有新增 unsafe，超大解码包也按三秒容量分块。
> 2. 重采样使用线性插值，未引入 rubato。设备默认采样率优先，只有源采样率不一致时才走该路径；接口已隔离，替换为 rubato 不影响引擎其余部分。

| ID | 任务 | 产出 | 天 |
|---|---|---|---|
| M2.1 | 解码抽象 | symphonia 0.6.1，多格式探测 | 1.0 |
| M2.2 | 输出与重采样 | cpal 0.18 + rubato；设备默认采样率优先 | 1.0 |
| M2.3 | 队列与 gapless | 预加载、ring buffer 无缝续接 | 1.0 |
| M2.4 | Seek 与缓冲 | HTTP Range、2–4s 预缓冲、断流续播 | 1.0 |
| M2.5 | 音量与淡入淡出 | f32 感知曲线；切歌 20ms 淡入淡出 | 0.5 |

验收：Phase 0 的 G3/G4 在真实网络下复现；按用户要求，连续播放标准调整为 80 分钟无漂移。

#### M3 应用外壳（4 天）

> **状态：首版完成（2026-11-12）**。GPUI 窗口可运行（debug 68MB），侧边栏导航、沉浸式渐变背景、专辑网格占位、播放条、队列抽屉均已落地；导航、播放/暂停、队列开关可交互。
>
> 本次补齐队列拖动/按钮排序、移除与清空待播；播放栏主动加载封面并提供 ID 回退。尚未完成：真实封面驱动背景、播放快捷键与真实媒体键验收。当前背景采用黑灰渐变；实际封面与拖动交互仍需可见界面验收。

> 0.1.5：专辑/歌单/搜索歌曲与队列使用可见行渲染，整份歌曲列表仅在播放或入队时复制；专辑和歌单头部保持可见，搜索歌曲区域独立滚动，队列增加“定位当前”。一万首本地列表已执行末尾滚动、点播和队列编辑检查；实际缩放及真实大曲库仍单独验收。

> 0.1.6：艺人 → 专辑逐层返回，保留艺人上下文；侧栏收起后提供图标导航并重算网格宽度。随机/循环改为矢量图标，移除操作说明小字及页副标题；歌词页只保留封面卡片与歌词，同步行按实际行边界居中，首尾留出半屏空间。真实《海底》同步歌词与封面已观察；《突然好想你》原始 FLAC 全解码和最后十秒播放通过，文件时长多约 44 ms 的曲尾 EOF 已按受限容差处理。

| ID | 任务 | 产出 | 天 |
|---|---|---|---|
| M3.1 | 主题与视觉 token | `theme.rs`、沉浸式背景（16×16 放大 + 缓存） | 1.0 |
| M3.2 | 侧边栏与路由 | 首页/艺人/专辑/歌单/收藏/搜索/设置 | 1.0 |
| M3.3 | 播放条 | 封面、标题、进度、音量、随机/循环 | 1.0 |
| M3.4 | 队列抽屉 | 拖动排序、删除、跳播 | 0.5 |
| M3.5 | 快捷键 | 空格/方向键/媒体键 | 0.5 |

#### M4 内容视图（3.5 天）

> **状态：首版完成（2026-11-12）**。连接页（服务器/用户名/密码 + IME 输入框）、后台 API worker、真实专辑列表、艺人列表/艺人页、专辑详情页、搜索（三类结果）均已接入。
>
> 已补：密码掩码（含 Unicode 光标映射）、真实封面图片、回车触发连接/搜索（组合输入期间不触发），以及专辑分页、网格与歌曲可见行渲染。

| ID | 任务 | 产出 | 天 |
|---|---|---|---|
| M4.1 | 专辑网格 | 虚拟化、懒加载封面、hover 播放 | 1.0 |
| M4.2 | 专辑详情 | 曲目表、播放全部、star | 1.0 |
| M4.3 | 艺人页 | 专辑列表 | 0.5 |
| M4.4 | 搜索页 | 三类结果 | 0.5 |
| M4.5 | 状态页 | 空/加载/错误/离线 | 0.5 |

#### M5 集成（3 天）

> **状态：核心播放已接入（2026-11-12）**。点击专辑「播放全部」或歌曲行即可播放，支持播放/暂停、上一首/下一首、队列抽屉点播、gapless 预取、进度显示、50%/90% scrobble。已用真实 Navidrome 拉流验证（MP3，0 underrun）。
>
> 2026-11-12 收尾：进度拖动预览并在松开时 seek、连续音量拖动、专辑 Star（albumId）与失败回滚、Windows HWND SMTC 绑定及中文元数据冒烟验证、音质/音量持久化、JSON + DPAPI 凭据保护均已落地。真实媒体键及 80 分钟真实服务稳定性仍待验收；本地素材长测最终结果见 [验收记录](03-acceptance.md)。

> 0.1.3：SMTC 复用已加载图片的内存字节，发布封面并丢弃过期转换；状态、标题与时间线由独立轮询同步，最小化窗口后程序化播放/暂停/上下首及 PNG 读回通过。硬件媒体键和系统面板视觉仍未验收。

| ID | 任务 | 产出 | 天 |
|---|---|---|---|
| M5.1 | Scrobble | 50% now-playing + 90% submission | 0.5 |
| M5.2 | Star / Unstar | 乐观更新 + 回滚 | 0.5 |
| M5.3 | Windows SMTC | 媒体键、系统弹窗、封面 | 1.0 |
| M5.4 | 设置与持久化 | JSON + Windows DPAPI（替代原 SQLite 计划） | 1.0 |

#### M6 打包（1.5 天）

> 2026-11-12：release 优化、图标、版本资源、Windows 绿色版 ZIP/SHA256、构建/粗测脚本与 GitHub Actions 已实现。GPUI 提供 DPI/系统兼容 manifest，应用只添加图标和版本资源，避免重复 manifest。NSIS/签名/公开发布与严格冷启动验收仍未完成。

- release profile：`lto = "thin"`、`codegen-units = 1`、`strip = true`、`panic = "abort"`
- 图标、manifest、DPI 声明
- 绿色版 zip + 可选 NSIS 安装包
- 冷启动与内存复测

**合计 20 天 ≈ 3.4 周。**

### 3.2 v1.0 验收

- [ ] 冷启动 < 1.0s，空闲内存 < 80MB，exe < 25MB
- [ ] 浏览、搜索、播放、队列、star、scrobble、SMTC 全部可用
- [x] WAV/FLAC/MP3/AAC/Vorbis 固定素材解码；Opus/WMA/APE/DSD 自动请求 MP3 转码（真实服务器格式矩阵待验收）
- [ ] 连续播放 80 分钟无明显内存增长、无音频漂移（真实服务；本地素材 120 分钟已通过）
- [ ] 断网 30 秒恢复后自动续播
- [ ] 中文搜索、中文标签、中文路径正常

### 3.3 每周节奏

| 周 | 内容 |
|---|---|
| W1 | M1 + M2.1–M2.2 |
| W2 | M2.3–M2.5 + M3.1–M3.2 |
| W3 | M3.3–M3.5 + M4 |
| W4 | M5 + M6 + 验收 |

---

## 4. Phase 2：v1.1（2–3 周）

| 任务 | 天 |
|---|---|
| 歌词：`getLyricsBySongId` + 滚动同步 + 回退 `getLyrics` | 2.0 |
| 歌单浏览与播放 | 1.5 |
| 设置页扩展（转码偏好、缓存上限、语言） | 1.0 |
| 浅色主题 | 1.0 |
| **TUI（ratatui 0.30 + crossterm 0.29）** | 4.0 |
| 测试与打磨 | 2.0 |

**TUI 子任务**：
- T1 三栏布局 + 键盘导航（1d）
- T2 复用 `melos-core` 的队列与播放（1d）
- T3 搜索与中文输入（终端原生 IME，0 风险）（0.5d）
- T4 可选封面（sixel/kitty，`ratatui-image`）（1d）
- T5 纯 TUI 构建（`--no-default-features`，< 6MB）（0.5d）

---

## 5. Phase 3：分发（3–5 天）

| 任务 | 天 |
|---|---|
| Windows：图标、版本信息、NSIS、代码签名 | 1.0 |
| macOS：`.app` + ad-hoc 签名 + Metal shader 编译进 CI | 1.0 |
| Linux：AppImage + Flatpak | 1.5 |
| 自动更新（可选，GitHub Releases 检查） | 1.0 |

---

## 6. 工程规范

### 6.1 CI 流水线

```
fmt       cargo fmt --all --check
clippy    cargo clippy --workspace --all-targets -- -D warnings
test      cargo test --workspace
deny      cargo deny check
arch      melos-core / melos-api / melos-player 不得依赖 gpui / ratatui
build     cargo build --release --locked
```

**架构检查**（防 R11）：

```bash
cargo tree -p melos-core -e normal | grep -E "gpui|ratatui" && exit 1 || exit 0
```

### 6.2 测试策略

| 层级 | 范围 | 工具 |
|---|---|---|
| 单元 | 队列、认证、能力矩阵、错误映射 | `cargo test` |
| 集成 | Subsonic 端点 | `wiremock` 或本地 axum mock |
| 音频 | 解码正确性、接缝 | 固定样例 + 输出写盘分析 |
| 手工 | IME、设备切换、格式矩阵 | 清单表格 |

### 6.3 依赖与许可

| 依赖 | 版本 | 许可 |
|---|---|---|
| gpui-unofficial / gpui-pre | 1.28.0-pre / 0.3.12 | Apache-2.0 |
| gpui-component | 0.9.1 | Apache-2.0 |
| symphonia | 0.6.1 | MPL-2.0 |
| cpal | 0.18.2 | Apache-2.0 |
| rubato | 5.0.1 | MIT / Apache-2.0 |
| rtrb | 0.4.1 | MIT / Apache-2.0 |
| reqwest | 0.13.5 | MIT / Apache-2.0 |
| tokio | 1.54.1 | MIT |
| rusqlite | 0.42.0 | MIT |
| ratatui | 0.30.2 | MIT |
| crossterm | 0.29.0 | MIT |

- 项目整体 GPL-3.0；以上均兼容
- MPL-2.0（symphonia）需保留文件级声明，不修改其源码即可
- `cargo deny` 配置：允许 Apache-2.0 / MIT / MPL-2.0 / BSD / ISC / Zlib / Unicode-3.0

### 6.4 代码约定

- Conventional Commits：`feat(player): ...`、`fix(api): ...`
- 每个 crate 单一职责；跨 crate 只通过公开类型通信
- 禁止在 `melos-core` 里出现 `use gpui` / `use ratatui`
- 音频回调内禁止 `alloc`、`lock`、`log`

---

## 7. 风险与回退

| 触发条件 | 回退动作 | 影响 |
|---|---|---|
| GPUI IME 不可用（A/C 均失败） | 切 egui，2 天验证 | UI 层重写，core 不变 |
| C 的体积/内存不可接受 | 选 A，自建 4 个基础控件 | +1 周 |
| gapless 无法做到 < 5ms | 接受 20ms，记录已知问题 | 体验降级 |
| 转码流 seek 受限 | 粗粒度 seek + 提示 | 仅冷门格式 |
| 进度落后 2 周 | 砍 v1.0 的 star/scrobble，只保留播放闭环 | 范围收缩 |

---

## 8. 附录：常用命令

```powershell
# 开发构建（必须经 vcvars）
cmd /c '"C:\Program Files (x86)\Microsoft Visual Studio\18\BuildTools\VC\Auxiliary\Build\vcvars64.bat" && cargo +1.99.0-x86_64-pc-windows-msvc check --workspace'

# 测试
cmd /c '"C:\Program Files (x86)\Microsoft Visual Studio\18\BuildTools\VC\Auxiliary\Build\vcvars64.bat" && cargo +1.99.0-x86_64-pc-windows-msvc test --workspace'

# 发布构建
cmd /c '"C:\Program Files (x86)\Microsoft Visual Studio\18\BuildTools\VC\Auxiliary\Build\vcvars64.bat" && cargo +1.99.0-x86_64-pc-windows-msvc build --release --locked'
```

---

## 9. 下一步

Phase 0 已结束。当前进入核心功能补齐与 Windows 预览验收，详细状态见 [项目进展](04-project-status.md)。

1. 随机/循环播放、播放快捷键已实现；继续实际交互验收。
2. 音频输出错误反馈、桌面设置页/终端 `e` 手动重连已实现。旧引擎两小时有 3 次欠载，新原子缓冲的 120 分钟本地素材复测零欠载、最大漂移 17 ms、平均私有内存增长约 1.55 MB。真实服务长测按用户要求使用 80 分钟，格式矩阵、硬件媒体键和物理拔插继续分别验收；`soak`、`server_check`、`track_check` 提供持续播放、格式样本及指定歌曲曲尾报告。
3. 专辑分页、曲库网格及歌曲/队列可见行渲染、按需加载与磁盘封面缓存已实现；继续真实大曲库与窗口缩放验收。
4. 歌单编辑、同步/纯文本歌词及 TUI 首版已实现，本地界面和交互终端检查通过；继续真实服务器验收及终端功能完善。
5. Windows 分发完善与严格性能验收；保持私有，按完成批次提交推送。
