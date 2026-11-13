Melos TUI 0.1.18 — 终端版

要求 Windows 10 1903+ / Windows 11（x64），并安装
Microsoft Visual C++ 2015-2022 Redistributable (x64)：
https://aka.ms/vs/17/release/vc_redist.x64.exe

在 Windows Terminal 或 PowerShell 中运行 melos-tui.exe。

首次使用
- 没有保存的连接时，启动会停在“音乐源”页
- 按 a 依次输入服务器、用户名、密码，Enter 提交并连接
- 已有源：↑/↓ 选择，Enter 连接，d 删除非当前源

按键
1 专辑 / 2 歌单 / 3 队列 / 4 歌词 / 5 全部歌曲 / 6 音乐源 / 7 艺人
/ 搜索歌曲与艺人（Enter 提交，Esc 取消）
↑/↓ 或 k/j 选择，Enter 打开专辑/歌单或播放歌曲
Space 播放/暂停，n/p 上一首/下一首，←/→ 快退/快进 5 秒
+/- 音量，s 随机，r 循环
歌曲页：a 加入队列，i 下一首播放
队列页：d 移除，J/K 下移/上移，D 清空待播
x 标记当前专辑/艺人，R 随机播放已标记范围（专辑页、艺人页）
L 加载更多（专辑页/全部歌曲页），t 切换歌词版本，e 重连音频
Esc 取消输入或返回上一级，q 或 Ctrl+C 退出

配置
保存在 %APPDATA%\melos\settings.json，密码由 Windows DPAPI 保护。
设置 MELOS_CONFIG_DIR 可指定独立配置目录，与桌面版完全隔离。

限制
暂不支持歌单编辑、艺人/专辑搜索和系统媒体键，请使用桌面版。
许可证 GPL-3.0-or-later，见 LICENSE 与 THIRD_PARTY_NOTICES.md。
