# KeepWindow

[![CI](https://github.com/NeterAlex/KeepWindow/actions/workflows/ci.yml/badge.svg?branch=main)](https://github.com/NeterAlex/KeepWindow/actions/workflows/ci.yml)
[![Latest release](https://img.shields.io/github/v/release/NeterAlex/KeepWindow?display_name=tag&sort=semver)](https://github.com/NeterAlex/KeepWindow/releases)

KeepWindow 是一个 Windows 窗口防误关闭工具。它可以禁用标准关闭命令，或用鼠标遮罩保护自绘标题栏的关闭按钮。

## 功能

- 系统关闭保护：禁用目标窗口系统菜单中的“关闭”项。
- 鼠标遮罩：拦截关闭按钮区域的单击和双击，支持 DPI 缩放、移动和调整大小。
- 应用规则：按程序路径、窗口类名和可选标题关键词保存设置，可为新窗口自动保护。
- 后台驻留：关闭或最小化主界面后继续保护，通过系统托盘返回或退出并恢复。
- 状态与恢复：显示实际生效的保护方式，支持单个或全部恢复，并保留恢复失败项供重试。

## 安装

从 [Releases](https://github.com/NeterAlex/KeepWindow/releases) 下载最新的 Windows x64 压缩包，解压后运行 `keep-window.exe`。

运行要求：Windows 10 1703 或更新版本，x64。目标程序以管理员身份运行时，KeepWindow 也可能需要以管理员身份运行。

## 使用

1. 打开目标程序，在列表中选择窗口。
2. 标准标题栏使用“系统关闭保护”；自绘标题栏使用“鼠标遮罩”，也可以同时启用。
3. 点击“开启保护”。使用鼠标遮罩时，确认半透明区域覆盖关闭按钮；必要时点击“校准遮罩”调整位置和大小。
4. 使用“应用设置”修改已保护窗口，或点击“解除保护”恢复窗口。

关闭、最小化主界面或按 `Alt+F4` 会隐藏 KeepWindow，保护继续运行。也可以直接启动到后台：

```powershell
.\keep-window.exe --background
```

## 应用规则

选中窗口并设置参数后，点击“保存为应用规则”。规则匹配完整程序路径（不区分大小写）和窗口类名（精确匹配）；填写“标题包含”后按不区分大小写的关键词匹配。自动保护默认关闭，多条规则命中时，带标题条件的规则优先。

规则保存在 `%LOCALAPPDATA%\KeepWindow\config.json`，跨运行生效。可在“更多 → 应用规则管理”中编辑、启用、禁用或删除规则。

## 保护范围

- 系统关闭保护依赖目标程序遵守 Windows 系统菜单状态；自绘标题栏可能绕过它。
- 鼠标遮罩只拦截覆盖区域内的鼠标点击，并且仅在目标窗口可见且位于前台时显示。
- `Alt+F4`、程序自己的“退出”命令、自定义快捷键、直接发送关闭消息或结束进程不受鼠标遮罩保护。
- 强制结束 KeepWindow 可能留下被禁用的系统关闭项；重启目标程序通常可以恢复。

## 从源码构建

需要 Rust stable MSVC 工具链、Visual Studio Build Tools 的“使用 C++ 的桌面开发”组件和 Windows SDK。

```powershell
git clone https://github.com/NeterAlex/KeepWindow.git
cd KeepWindow
cargo build --release
.\target\release\keep-window.exe
```

日常开发可使用 `cargo check`。提交前建议运行：

```powershell
cargo fmt --check
cargo clippy --locked --all-targets --all-features -- -D warnings
cargo test --locked --all-targets --all-features -- --test-threads=1
```

## 许可证

[MIT License](LICENSE)

交互思路参考 [PettterWang/NOCLOSE](https://github.com/PettterWang/NOCLOSE)。
