# KeepWindow

Windows 窗口防误关闭工具。为选定窗口禁用系统关闭操作，或用鼠标遮罩覆盖关闭按钮，减少工作时的误触。

## 功能

- **系统关闭保护**：禁用目标窗口系统菜单中的“关闭”项，适用于采用标准关闭处理的程序。
- **鼠标遮罩**：拦截关闭按钮区域的单击和双击，支持手动调整尺寸、偏移及 DPI 缩放。
- **窗口搜索**：按名称、窗口类名、PID 或窗口句柄筛选，分别管理多个窗口的保护状态。
- **后台驻留**：关闭或最小化主界面后继续保护，通过系统托盘返回界面或退出。
- **一键恢复**：解除单个或全部窗口的保护，正常退出时恢复已修改的关闭状态。

## 构建与运行

运行环境：Windows 10 1703 或更新版本，x64。

从源码构建需要：

- Rust 稳定版，使用 `x86_64-pc-windows-msvc` 工具链。
- Visual Studio Build Tools 的“使用 C++ 的桌面开发”组件及 Windows SDK。

```powershell
git clone https://github.com/NeterAlex/KeepWindow.git
cd KeepWindow
cargo build --release
.\target\release\keep-window.exe
```

图标和版本信息自动嵌入 EXE，无需单独分发图标文件。

## 使用方法

1. 打开目标程序，在 KeepWindow 中点击“刷新”并选择窗口。
2. 对标准标题栏启用“系统关闭保护”；对自绘标题栏启用“鼠标遮罩”。两种方式可以组合使用，没有系统菜单的窗口使用鼠标遮罩。
3. 点击“开启保护”。使用遮罩时，切换到目标窗口，确认绿色半透明区域覆盖关闭按钮。
4. 点击“解除保护”恢复所选窗口，或通过“更多 → 恢复全部窗口”批量恢复。

遮罩默认宽 46、高 32 DIP，按目标窗口 DPI 换算为实际像素。“右侧内缩”和“顶部下移”的正值向窗口内部移动，负值反向移动。调整已保护窗口的参数时，先解除保护，修改后再开启。

### 后台运行与退出

关闭、最小化主界面或按 Alt+F4 会隐藏 KeepWindow，保护继续运行。点击系统托盘图标可返回界面；图标可能位于托盘折叠菜单中。

直接启动到后台：

```powershell
.\target\release\keep-window.exe --background
```

通过“更多 → 退出并恢复”或托盘菜单“退出并恢复保护”结束程序。恢复失败时，主界面会显示错误并保留相关窗口，便于重试。

保护配置仅在本次运行中保留；`--background` 不设置开机自启，也不恢复上次的保护列表。强制结束进程可能留下禁用的系统关闭项，此时重启目标程序可恢复。

## 保护范围

| 关闭途径 | 系统关闭保护 | 鼠标遮罩 |
| --- | --- | --- |
| 标准标题栏关闭按钮 / 系统菜单“关闭” | 适用于标准关闭处理 | 仅拦截遮罩区域内的鼠标点击 |
| 自绘标题栏关闭按钮 | 取决于程序是否遵守系统菜单状态 | 遮罩覆盖按钮时可拦截点击 |
| Alt+F4 | 取决于目标程序的关闭处理 | 不拦截键盘 |
| 程序菜单“退出”、自定义快捷键 | 可绕过保护 | 不保护 |
| 直接发送关闭消息、程序自行退出、结束进程 | 不保护 | 不保护 |

Windows Terminal 等自绘标题栏程序的关闭按钮可以绕过系统菜单禁用状态，建议使用鼠标遮罩，并检查实际覆盖位置。遮罩仅在目标窗口位于前台且可见时显示。

目标程序以管理员身份运行时，KeepWindow 也可能需要以管理员身份运行。

## 开发与贡献

项目使用 Rust、GPUI Kit 和 windows-rs。提交改动前，请在 Windows 上运行：

```powershell
cargo fmt --check
cargo test --locked --all-targets --all-features -- --test-threads=1
cargo clippy --locked --all-targets --all-features -- -D warnings
cargo build --locked --release --all-features
```

测试覆盖窗口选择与搜索、保护状态恢复、鼠标拦截及托盘生命周期。涉及第三方程序或 DPI 的改动，请补充实际应用中的验证结果。

图标源文件位于 [`assets/`](assets/)。修改 SVG 后，使用 [uv](https://docs.astral.sh/uv/) 重新生成图标：

```powershell
uv run scripts/generate_icon.py
```

欢迎通过 [Issues](https://github.com/NeterAlex/KeepWindow/issues) 报告问题，或提交 Pull Request。兼容性问题请附上 Windows 版本、目标应用及版本、显示缩放比例、保护方式和复现步骤。

## GitHub Actions

- **CI**：推送到 `main`、提交 Pull Request 或手动触发时，检查格式，并在 Windows Server 2022 的 x64 MSVC 环境中运行 Clippy、串行测试和 Release 构建。测试操作原生窗口及鼠标钩子，需使用 `--test-threads=1`。
- **构建产物**：通过检查后，上传 `keep-window-windows-x64` artifact，保留 14 天。其中包含携带 EXE、README 和许可证的 ZIP 包，以及对应的 SHA-256 校验文件。
- **Release**：推送与 `Cargo.toml` 版本一致的 `v*` 标签时，复用全部 CI 检查，并将 ZIP 和校验文件发布到 GitHub Releases。带预发布后缀的版本自动标记为 prerelease。
- **依赖维护**：Dependabot 每周检查 Rust 依赖和 GitHub Actions 更新。Actions 固定到完整提交 SHA；普通检查只授予读取权限，发布任务单独授予写入权限。

例如，当前 `Cargo.toml` 版本为 `0.2.0` 时，可用以下命令触发发布：

```powershell
git tag v0.2.0
git push origin v0.2.0
```

发布流程使用 GitHub 自动提供的 `GITHUB_TOKEN`，无需额外配置发布密钥。

## 致谢

交互思路参考 [PettterWang/NOCLOSE](https://github.com/PettterWang/NOCLOSE)。

## 许可证

[MIT License](LICENSE)
