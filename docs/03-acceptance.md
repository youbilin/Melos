# v1.0 验收记录

更新：2026-11-12。只将实际执行过的检查记录为通过。

## 自动化检查

- `cargo fmt --all --check`：通过。
- `cargo test --workspace --locked`：31 个测试通过，1 个长测试显式忽略；音频解码和 seek 使用仓库自带素材，不再依赖环境变量或静默跳过。
- `cargo clippy --workspace --all-targets --locked -- -D warnings`：通过。
- 格式素材：生成的 440 Hz 信号，WAV、FLAC、MP3、AAC/M4A、Vorbis/OGG。
- Gapless：FLAC → MP3 的输出缓冲样本、曲目边界与接缝能量检查通过；不等同于全部真实音频设备场景。
- HTTP：本地服务端验证 Range seek、截断后重试续传，不丢失或重复字节。
- 30 秒断流恢复长测试：显式执行通过，实际用时 30.06 秒。
- 收藏：验证专辑使用 `albumId`，服务端拒绝时保留错误与请求标识供 UI 回滚。
- Windows DPAPI：中文测试密码加密/解密通过。

## Windows 运行检查

通过设置 `MELOS_SMOKE_REPORT` 为输出 JSON 路径并运行 `melos.exe` 执行冒烟；约一秒后自动退出。检查不连接音乐服务器、不修改用户设置。

- 原生窗口 SMTC 绑定：通过。
- 中文曲目/艺人元数据与时间线发布：通过。
- 默认音频设备创建：通过。

## 发布前待验收

- [ ] 实际媒体键：播放/暂停、上一首/下一首，系统媒体面板。
- [ ] 拖动进度与音量的实际鼠标交互、中文密码与搜索、中文路径。
- [ ] 真实 Navidrome：FLAC/MP3/AAC/Vorbis 播放、冷门格式转码、转码流 seek 限制。
- [ ] 连续播放 2 小时无内存增长或音频漂移；暂停恢复、快速切歌、设备断开。
- [ ] 完整 release：exe < 25 MB、冷启动 < 1.0 秒、空闲内存 < 80 MB。

原 Phase 0 性能数据来自 spike，不作为完整应用的性能结论。
