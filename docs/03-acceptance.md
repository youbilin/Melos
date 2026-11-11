# v1.0 验收记录

更新：2026-11-12。只将实际执行过的检查记录为通过。

## 自动化检查

- `cargo fmt --all --check`：通过。
- `cargo test --workspace --locked`：31 个测试通过，1 个长测试显式忽略；音频解码和 seek 使用仓库自带素材，不再依赖环境变量或静默跳过。
- `cargo clippy --workspace --all-targets --locked -- -D warnings`：通过。
- 格式素材：生成的 440 Hz 信号，WAV、FLAC、MP3、AAC/M4A、Vorbis/OGG。
- Gapless：FLAC → MP3 的输出缓冲样本、曲目边界与接缝能量检查通过；不等同于全部真实音频设备场景。
- WAV/FLAC/MP3 一秒素材均解码为准确的 44,100 帧；修复 Symphonia 已裁剪后再次裁剪的问题。
- HTTP：本地服务端验证 Range seek、截断后重试续传，不丢失或重复字节。
- 30 秒断流恢复长测试：显式执行通过，实际用时 30.06 秒。
- 收藏：验证专辑使用 `albumId`，服务端拒绝时保留错误与请求标识供 UI 回滚。
- Windows DPAPI：中文测试密码加密/解密通过。

## Windows 运行检查

通过设置 `MELOS_SMOKE_REPORT` 为输出 JSON 路径并运行 `melos.exe` 执行冒烟；约一秒后自动退出。检查不连接音乐服务器、不修改用户设置。

- 原生窗口 SMTC 绑定：通过。
- 中文曲目/艺人元数据与时间线发布：通过。
- 默认音频设备创建：通过。
- 实际 WASAPI 播放一秒 FLAC → 一秒 MP3：两曲均播放完整，最终位置 1,000 ms，underrun 为 0（短时素材检查）。

## Windows 绿色版与性能粗测

- `packaging/build-windows.ps1`：通过；MSVC 自动定位、release 构建、ZIP/SHA256 生成。
- 应用图标、0.1.0 版本资源已嵌入；DPI/系统兼容 manifest 由 GPUI 提供。
- 完整应用 release exe：约 **13.24 MB**；ZIP：**5,886,756 字节**。
- ZIP 校验和、解压前后 exe 哈希一致性：通过。
- 解压后的 exe：隐藏启动、SMTC 绑定、中文元数据发布、音频设备创建均通过。
- `packaging/measure-windows.ps1` 三次窗口初始化报告时间：**1,040.9 / 268.8 / 269.6 ms**；对应工作集 **47.54 / 47.96 / 48.07 MB**。
- 以上为隐藏窗口初始化与短时空闲场景；不代表可见首帧、受控冷启动、大曲库或长时间播放内存。首次仍高于 1 秒，冷启动验收不标为通过。
- 支持 Windows 10 1903+ / Windows 11 x64，需要系统安装 Visual C++ Redistributable。
- GitHub Actions 配置格式、测试、30 秒恢复、Clippy、底层依赖检查与 Windows 绿色版构建，产物保留在私有仓库中；不发布 GitHub Release。

## 发布前待验收

- [ ] 实际媒体键：播放/暂停、上一首/下一首，系统媒体面板。
- [ ] 拖动进度与音量的实际鼠标交互、中文密码与搜索、中文路径。
- [ ] 真实 Navidrome：FLAC/MP3/AAC/Vorbis 播放、冷门格式转码、转码流 seek 限制。
- [ ] 连续播放 2 小时无内存增长或音频漂移；暂停恢复、快速切歌、设备断开。
- [ ] 完整 release：exe < 25 MB、冷启动 < 1.0 秒、空闲内存 < 80 MB。

原 Phase 0 性能数据来自 spike，不作为完整应用的性能结论。
