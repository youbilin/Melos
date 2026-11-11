# Project workflow

- Repository: https://github.com/youbilin/Melos (private). Keep it private unless the user asks otherwise.
- The user authorizes committing and pushing project changes. After each coherent, verified batch, create a descriptive commit and push to `origin/main`. Do not leave completed changes uncommitted.
- Never commit credentials, local settings, build output, logs, or distribution archives.
- Use the pinned Rust toolchain and the MSVC developer environment on Windows. Run formatting, workspace tests, and Clippy for code changes.
- Keep README and the implementation plan consistent with actual code. Distinguish implemented functionality from manually verified acceptance criteria.
- Windows is the release target. Keep domain and player crates independent of GPUI.
- Do not publish releases or change repository visibility without a user request.
