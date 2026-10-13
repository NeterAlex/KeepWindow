# KeepWindow

用 Rust 实现的 Windows 窗口防误关闭工具，参考 [PettterWang/NOCLOSE](https://github.com/PettterWang/NOCLOSE) 的交互思路。主界面使用 `gpui-kit`，窗口枚举、系统菜单和鼠标遮罩通过 `windows` crate 调用 Win32 API。

## GUI 方案结论

**本项目采用 `gpui-kit` 作为 GUI，`windows-rs` 作为 Windows 能力层。** GPUI Kit 基于 GPUI，提供 Rust 原生、可组合的按钮、复选框、输入框和列表组件，适合这个需要持续刷新窗口列表、编辑遮罩参数的小工具。Win32 仍然是保护核心的必要依赖：只有它能可靠访问目标窗口的系统菜单、DWM 标题栏按钮坐标、DPI 和跨进程 HWND。

| 方案 | 优点 | 本项目的取舍 |
| --- | --- | --- |
| `gpui-kit` + Win32 | Rust 原生组件；布局和交互开发效率高；保护核心仍可直接使用 HWND、DPI 和 Win32 消息 | **当前采用**。需要接受 GPUI 的渲染依赖，并保留 Windows 专用保护代码 |
| `windows-rs` + Win32 控件 | 原生控件；直接管理 HWND、DPI、置顶和不激活窗口；依赖较少 | 可作为极简原生界面方案，但布局和消息循环样板代码更多 |
| `egui` / `eframe` | 纯 Rust 即时模式 UI，工具界面开发快 | 也能实现本项目 UI，但本项目已选择 GPUI Kit；保护核心仍需要 Win32 |
| Slint | 声明式界面，适合精细视觉设计和更复杂的产品界面；提供多个渲染后端 | 适合后续产品化，目前功能规模用不上其主要优势；需按发布方式核对许可证 |
| `native-windows-gui` | 封装原生控件，减少部分 Win32 样板代码 | crates.io 最新稳定版仍为 1.0.13，发布于 2022 年；使用前需要评估维护与工具链兼容性 |
| Tauri | Web 前端生态成熟，适合已有 Web UI 的团队 | 引入 WebView 与前后端通信，对目前几个控件的小工具收益有限；窗口保护仍要调用 Win32 |

上述结论针对当前功能，不以跨平台或复杂视觉界面为目标。若后续更换 GUI，只需保留 `protection.rs`、`mouse.rs` 和 `window.rs` 的 Windows 能力层。

## 与 NOCLOSE 的关系

阅读参考仓库的 `NOCLOSE.cs` 和 `NOCLOSE.Designer.cs` 后，可以确认其核心是定时跟踪目标窗口，在右上角显示无边框置顶窗口，覆盖关闭按钮所在区域。它不是通用的关闭消息拦截器。

本项目独立实现两种可以组合的方式：

1. **禁用系统关闭**：通过 `GetSystemMenu` 获取系统菜单，以 `EnableMenuItem(..., SC_CLOSE, MF_BYCOMMAND | MF_GRAYED)` 禁用关闭项。适合标准标题栏及使用默认关闭处理的程序。目标自行重新启用关闭项时，工具会定期重新禁用。
2. **鼠标遮罩**：创建带 `WS_EX_LAYERED | WS_EX_NOACTIVATE | WS_EX_TOOLWINDOW | WS_EX_TOPMOST` 的半透明窗口，消费关闭按钮区域的鼠标点击。适合自绘标题栏，位置和尺寸可以手动调整。没有设置会使鼠标穿透的 `WS_EX_TRANSPARENT`。
3. **低级鼠标钩子**：使用 `WH_MOUSE_LL` 在目标进程收到输入前拦截遮罩区域的按下、抬起和双击消息。这样双击记事本关闭按钮时，第二次按下也会被拦截，不会出现“单击无效但双击关闭”的漏洞；按下后即使指针短暂移出区域，释放消息也会继续被消费。

本项目不修改目标窗口的标题。保护前保存系统关闭项的禁用状态，恢复时只还原该项，不重置整个系统菜单。目标窗口上附加一个唯一属性；属性随窗口销毁，用于降低 HWND 复用造成误操作的风险。

## 使用

要求：Windows 10 1703 或更新版本。当前构建产物为 Windows x64。

```powershell
cargo run --release
```

或者直接运行 `target\release\keep-window.exe`。

1. 打开目标程序，点击“刷新”，选择对应窗口。表格按窗口名称、PID、窗口句柄和保护状态分列；可搜索名称、窗口类名、PID 或十六进制句柄，多个关键词以空格分隔。
2. 对普通标题栏保留“系统关闭保护”，点击“开启保护”。
3. 如果关闭按钮由程序自行绘制，启用“鼠标遮罩”。没有标准系统菜单的窗口需要取消“系统关闭保护”。
4. 遮罩默认宽 46、高 32 DIP；可调整“右侧内缩”和“顶部下移”。正值向窗口内部移动，负值反向移动，实际像素按目标 DPI 换算。
5. 切换到目标窗口检查绿色半透明遮罩是否覆盖关闭按钮。切换窗口会保留各自的待应用参数；已保护窗口展示生效参数。需要调整时，先恢复该窗口，修改参数，再开启保护。
6. 所选窗口已保护时，主按钮切换为“解除保护”。也可通过“更多 → 恢复全部窗口”解除全部保护。“更多 → 退出并恢复”或托盘菜单的退出操作恢复全部窗口并结束程序；恢复失败时会显示主界面，保留未恢复的窗口以便重试。

刷新后按窗口身份保留选择；窗口关闭或被搜索条件隐藏时清空选择，已经开启的保护继续运行。表格支持方向键切换选择、Esc 取消选择和拖动列边界调整列宽。搜索与刷新共用顶栏，刷新按钮左侧的灯泡图标提供使用提示。遮罩参数仅在启用鼠标遮罩时显示；悬停参数标签可查看范围与方向说明，悬停 DIP 可查看缩放说明。底部单行状态栏左侧显示窗口总数、搜索匹配数和保护数量，右侧显示最近操作结果。

## 后台驻留与系统托盘

- 启动后，通知区域会显示绿色盾牌图标；Windows 可能将其放在托盘折叠菜单中。
- 点击标题栏的最小化或关闭按钮、按 Alt+F4，都会隐藏主界面并从任务栏移除，窗口保护继续运行。
- 单击或双击托盘图标可重新显示主界面；通过键盘选择托盘图标也可打开界面。重新显示保留最大化状态和已开启的保护。
- 右键托盘图标可选择“显示主界面”“恢复全部窗口”“退出并恢复保护”。退出时移除托盘图标并释放鼠标钩子和遮罩。
- Windows Explorer 重启后，程序会响应 `TaskbarCreated` 消息重新注册托盘图标。初次创建托盘失败时，主界面保持可访问并显示错误。

直接启动到后台：

```powershell
.\target\release\keep-window.exe --background
# 开发时也可使用：
cargo run --release -- --background
```

后台驻留保持本次进程中已经开启的保护。`--background` 不设置开机自启，也不保存或恢复上一次运行的保护列表。

## 保护范围

| 关闭途径 | 系统关闭保护 | 鼠标遮罩 |
| --- | --- | --- |
| 标准标题栏 X / 系统菜单“关闭” | 适用于采用标准系统关闭处理的窗口 | 遮罩覆盖、显示时可拦截点击 |
| Alt+F4 | 标准处理路径通常会受系统关闭项禁用影响；程序自定义处理可能绕过 | 不拦截键盘 |
| 自绘标题栏的 X | 程序可能不检查系统关闭项 | 调整遮罩后可拦截点击 |
| 程序菜单“退出”、自定义快捷键 | 不保证 | 不保护 |
| 直接发送 `WM_CLOSE` / 程序自行销毁窗口 / 结束进程 | 不保护 | 不保护 |

保护通过独立的 GPUI 异步定时任务每约 100 ms 更新，不依赖主窗口绘制，主界面隐藏时仍会重新禁用系统关闭项并跟踪遮罩。遮罩只在目标是前台窗口、可见、未最小化且未被 DWM 隐藏时显示。低级鼠标钩子在输入到达目标前再次检查 HWND、窗口身份和遮罩矩形；窗口移动或切换前台时仍应通过实机检查遮罩位置。遮罩模式是防误触辅助，不能阻止程序菜单、快捷键或直接发送 `WM_CLOSE`。

浏览器、Electron、UWP 和其他自绘窗口需要逐个验证布局和权限，不声称普遍兼容。目标以管理员身份运行时，KeepWindow 通常也需要相应权限；访问失败会提示。

请通过界面或托盘菜单的“退出”正常结束 KeepWindow；主窗口关闭按钮只隐藏界面。若强制终止 KeepWindow，遮罩会随进程消失，但对目标系统菜单的修改可能保留。此时重启目标程序可以恢复；重新开启并解除保护会保留当时读到的禁用状态，不能找回已经丢失的原状态。

## 开发与验证

```powershell
cargo fmt --check
cargo test
cargo clippy --all-targets -- -D warnings
cargo build --release
```

11 项自动测试覆盖系统菜单禁用与恢复、原先禁用状态的保留、重新启用关闭项后的处理、目标销毁后的清理、DPI 与负坐标计算、遮罩不激活且不穿透鼠标，以及双击和指针移出后的鼠标消息过滤。选择与过滤测试验证列表重排和标题变化后的选择保持、关闭或句柄复用后的清除，以及多字段搜索。托盘测试验证关闭与最小化不销毁窗口、菜单命令分发和资源释放；独立进程测试启动实际 GPUI 程序，验证 `--background`、隐藏窗口的异步任务、托盘恢复、最大化状态保留及正常退出。另一项独立进程测试验证跨进程菜单修改，并确认直接发送 `WM_CLOSE` 可以绕过保护。不同第三方程序的关闭路径、托盘右键菜单的实际点击和鼠标遮罩仍需要实机验证。

代码结构：

- `src/app.rs`：GPUI Kit 主窗口、后台定时刷新、托盘动作和安全退出。
- `src/window.rs`：窗口枚举、标题与进程信息、窗口身份校验。
- `src/protection.rs`：系统菜单修改、还原、鼠标遮罩、DWM / DPI 坐标计算。
- `src/mouse.rs`：进程级 `WH_MOUSE_LL` 钩子和单击、双击输入过滤。
- `src/tray.rs`：Win32 通知图标、托盘菜单、窗口隐藏/恢复和 Explorer 重启处理。
- `tests/tray_runtime.rs`：实际 GPUI 进程的后台与托盘生命周期测试。

## 参考资料

- [NOCLOSE 核心实现](https://github.com/PettterWang/NOCLOSE/blob/main/NOCLOSE.cs)
- [Microsoft windows-rs](https://github.com/microsoft/windows-rs)
- [GetSystemMenu 文档](https://learn.microsoft.com/en-us/windows/win32/api/winuser/nf-winuser-getsystemmenu)
- [EnableMenuItem 文档](https://learn.microsoft.com/en-us/windows/win32/api/winuser/nf-winuser-enablemenuitem)
- [Layered Windows 与鼠标命中](https://learn.microsoft.com/en-us/windows/win32/winmsg/window-features#layered-windows)
- [gpui-kit 文档](https://docs.rs/gpui-kit/latest/gpui_kit/)
- [Slint 后端与渲染器](https://docs.slint.dev/latest/docs/slint/guide/backends-and-renderers/backend_winit)
- [native-windows-gui 发布信息](https://crates.io/crates/native-windows-gui)
