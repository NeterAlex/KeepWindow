use std::time::Duration;

use gpui_kit::{
    AppContext as _, Context, Entity, IntoElement, ParentElement as _, Render, SharedString,
    Styled as _, Window, WindowOptions,
    base::StyledExt,
    component::{
        ActiveTheme,
        button::Button,
        button::ButtonVariants,
        checkbox::Checkbox,
        input::{Input, InputState},
        list::ListItem,
        scroll::ScrollableElement,
    },
    div, px,
};
use raw_window_handle::{HasWindowHandle, RawWindowHandle};
use windows::Win32::Foundation::HWND;

use crate::{
    protection::{Options, Protection},
    tray::{Action as TrayAction, Tray},
    window::{self, WindowInfo},
};

pub struct KeepWindow {
    windows: Vec<WindowInfo>,
    protections: Vec<Protection>,
    selected: Option<usize>,
    status: SharedString,
    system_menu: bool,
    mouse_shield: bool,
    width: Entity<InputState>,
    height: Entity<InputState>,
    right_offset: Entity<InputState>,
    top_offset: Entity<InputState>,
    tray: Option<Tray>,
    _refresh_task: gpui_kit::Task<()>,
    _quit_subscription: gpui_kit::Subscription,
}

impl KeepWindow {
    fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        window.set_window_title("KeepWindow");
        let tray = HasWindowHandle::window_handle(window)
            .map_err(|error| format!("无法获取主窗口句柄：{error}"))
            .and_then(|handle| match handle.as_raw() {
                RawWindowHandle::Win32(handle) => Tray::new(HWND(handle.hwnd.get() as *mut _)),
                _ => Err("主窗口不是 Windows 窗口。".into()),
            });
        let view = cx.weak_entity();
        window.on_window_should_close(cx, move |_, cx| {
            view.update(cx, |this, cx| {
                if let Some(tray) = &this.tray {
                    tray.hide();
                } else {
                    this.exit(cx);
                }
            })
            .ok();
            false
        });
        let width = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder("宽度")
                .default_value("46")
        });
        let height = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder("高度")
                .default_value("32")
        });
        let right_offset = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder("右侧内缩")
                .default_value("0")
        });
        let top_offset = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder("顶部下移")
                .default_value("0")
        });
        let quit_subscription = cx.on_app_quit(|this, _cx| {
            for protection in &mut this.protections {
                let _ = protection.restore();
            }
            this.protections.clear();
            this.tray.take();
            async {}
        });
        let mut this = Self {
            windows: Vec::new(),
            protections: Vec::new(),
            selected: None,
            status: "正在读取窗口列表…".into(),
            system_menu: true,
            mouse_shield: true,
            width,
            height,
            right_offset,
            top_offset,
            tray: None,
            _refresh_task: cx.spawn(async move |this, cx| {
                loop {
                    cx.background_executor()
                        .timer(Duration::from_millis(100))
                        .await;
                    if this
                        .update(cx, |this, cx| {
                            this.tick(cx);
                            this.process_tray_actions(cx);
                        })
                        .is_err()
                    {
                        break;
                    }
                }
            }),
            _quit_subscription: quit_subscription,
        };
        this.refresh(cx);
        match tray {
            Ok(tray) => {
                this.tray = Some(tray);
            }
            Err(error) => this.status = error.into(),
        }
        this
    }

    fn refresh(&mut self, cx: &mut Context<Self>) {
        match window::enumerate() {
            Ok(windows) => {
                self.windows = windows;
                self.selected = self.selected.filter(|i| *i < self.windows.len());
                self.status = format!(
                    "找到 {} 个窗口；正在保护 {} 个。",
                    self.windows.len(),
                    self.protections.len()
                )
                .into();
                cx.notify();
            }
            Err(error) => self.status = format!("刷新失败：{error}").into(),
        }
    }

    fn tick(&mut self, cx: &mut Context<Self>) {
        let mut ended = Vec::new();
        self.protections
            .retain_mut(|protection| match protection.tick() {
                Ok(()) => true,
                Err(error) => match protection.restore() {
                    Ok(()) => {
                        ended.push(error);
                        false
                    }
                    Err(e) => {
                        ended.push(format!("{error}；恢复失败：{e}"));
                        true
                    }
                },
            });
        if !ended.is_empty() {
            self.refresh(cx);
            self.status = ended.join("；").into();
        }
    }

    fn process_tray_actions(&mut self, cx: &mut Context<Self>) {
        let actions = self
            .tray
            .as_ref()
            .map(Tray::take_actions)
            .unwrap_or_default();
        for action in actions {
            match action {
                TrayAction::Show => {
                    if let Some(tray) = &self.tray {
                        tray.show();
                    }
                    self.refresh(cx);
                }
                TrayAction::RestoreAll => self.restore_all(cx),
                TrayAction::Exit => {
                    self.exit(cx);
                    break;
                }
            }
        }
    }

    fn minimize_to_tray(&mut self, cx: &mut Context<Self>) {
        if let Some(tray) = &self.tray {
            tray.hide();
        } else {
            self.status = "系统托盘不可用，主界面将保持可见。".into();
            cx.notify();
        }
    }

    fn exit(&mut self, cx: &mut Context<Self>) {
        self.restore_all(cx);
        if self.protections.is_empty() {
            self.tray.take();
            cx.quit();
        } else if let Some(tray) = &self.tray {
            tray.show();
        }
    }

    fn selected_window(&self) -> Option<WindowInfo> {
        self.selected.and_then(|i| self.windows.get(i)).cloned()
    }

    fn options(&self, cx: &gpui_kit::App) -> Result<Options, SharedString> {
        let number =
            |input: &Entity<InputState>, name: &str, min: i32, max: i32| -> Result<i32, String> {
                input
                    .read(cx)
                    .value()
                    .trim()
                    .parse::<i32>()
                    .ok()
                    .filter(|v| (min..=max).contains(v))
                    .ok_or_else(|| format!("{name}必须为 {min} 到 {max} 之间的整数。"))
            };
        let mut options = Options {
            system_menu: self.system_menu,
            mouse_shield: self.mouse_shield,
            ..Options::default()
        };
        if self.mouse_shield {
            options.width = number(&self.width, "遮罩宽度", 1, 512)?;
            options.height = number(&self.height, "遮罩高度", 1, 512)?;
            options.right_offset = number(&self.right_offset, "右侧内缩", -512, 512)?;
            options.top_offset = number(&self.top_offset, "顶部下移", -512, 512)?;
        }
        Ok(options)
    }

    fn protect(&mut self, cx: &mut Context<Self>) {
        let Some(target) = self.selected_window() else {
            self.status = "请先选择要保护的窗口。".into();
            return;
        };
        if self
            .protections
            .iter()
            .any(|p| p.target.hwnd == target.hwnd)
        {
            self.status = "所选窗口已受保护，请先恢复后再修改参数。".into();
            return;
        }
        match self.options(cx) {
            Ok(options) => match Protection::start(target.clone(), options) {
                Ok(p) => {
                    self.protections.push(p);
                    self.status = format!("已保护“{}”。", target.title).into();
                }
                Err(e) => self.status = e.into(),
            },
            Err(e) => self.status = e,
        }
        cx.notify();
    }

    fn restore_selected(&mut self, cx: &mut Context<Self>) {
        let Some(target) = self.selected_window() else {
            self.status = "请先选择要恢复的窗口。".into();
            return;
        };
        let Some(index) = self
            .protections
            .iter()
            .position(|p| p.target.hwnd == target.hwnd)
        else {
            self.status = "所选窗口尚未开启保护。".into();
            return;
        };
        match self.protections[index].restore() {
            Ok(()) => {
                self.protections.remove(index);
                self.status = format!("已恢复“{}”。", target.title).into();
            }
            Err(e) => self.status = e.into(),
        }
        cx.notify();
    }

    fn restore_all(&mut self, cx: &mut Context<Self>) {
        let mut errors = Vec::new();
        self.protections.retain_mut(|p| match p.restore() {
            Ok(()) => false,
            Err(e) => {
                errors.push(e);
                true
            }
        });
        self.status = if errors.is_empty() {
            "已恢复全部窗口。".into()
        } else {
            format!("部分窗口恢复失败：{}", errors.join("；")).into()
        };
        cx.notify();
    }

    fn badge(&self, hwnd: windows::Win32::Foundation::HWND) -> &'static str {
        match self
            .protections
            .iter()
            .find(|p| p.target.hwnd == hwnd)
            .map(|p| (p.options.system_menu, p.options.mouse_shield))
        {
            Some((true, true)) => "系统 + 遮罩",
            Some((true, false)) => "系统保护",
            Some((false, true)) => "鼠标遮罩",
            Some((false, false)) => "未保护",
            None => "未保护",
        }
    }
}

impl Render for KeepWindow {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme();
        let rows = self.windows.iter().enumerate().map(|(i, item)| {
            let selected = self.selected == Some(i);
            let label = item.label();
            let badge = self.badge(item.hwnd);
            ListItem::new(i)
                .w_full()
                .p_2()
                .rounded_md()
                .border_1()
                .border_color(if selected { theme.accent } else { theme.border })
                .bg(if selected {
                    theme.secondary
                } else {
                    theme.background
                })
                .cursor_pointer()
                .on_click(cx.listener(move |this, _, _, cx| {
                    this.selected = Some(i);
                    cx.notify();
                }))
                .child(
                    div().flex().justify_between().child(label).child(
                        div()
                            .text_sm()
                            .text_color(theme.muted_foreground)
                            .child(badge),
                    ),
                )
        });
        div().size_full().v_flex().gap_3().p_5().bg(theme.background).text_color(theme.foreground).font_family("Microsoft YaHei UI")
            .child(div().v_flex().gap_1().child(div().text_2xl().child("KeepWindow")).child(div().text_sm().text_color(theme.muted_foreground).child("防止重要窗口被误点关闭")))
            .child(div().flex().gap_2().child(Button::new("refresh").label("刷新列表").on_click(cx.listener(|this, _, _, cx| this.refresh(cx)))).child(Button::new("restore-all").label("恢复全部").on_click(cx.listener(|this, _, _, cx| this.restore_all(cx)))).child(Button::new("hide").label("最小化到托盘").on_click(cx.listener(|this, _, _, cx| this.minimize_to_tray(cx)))).child(Button::new("exit").label("退出").on_click(cx.listener(|this, _, _, cx| this.exit(cx)))))
            .child(div().flex_1().min_h(px(160.)).overflow_y_scrollbar().children(rows))
            .child(div().v_flex().gap_2().child(Checkbox::new("system-menu").label("禁用系统关闭（标准标题栏）").checked(self.system_menu).on_change(cx.listener(|this, v, _, cx| { this.system_menu = *v; cx.notify(); }))).child(Checkbox::new("mouse-shield").label("鼠标遮罩（拦截单击、双击和三击）").checked(self.mouse_shield).on_change(cx.listener(|this, v, _, cx| { this.mouse_shield = *v; cx.notify(); }))))
            .child(div().flex().gap_2().children([Input::new(&self.width), Input::new(&self.height), Input::new(&self.right_offset), Input::new(&self.top_offset)]))
            .child(div().flex().gap_2().child(Button::new("protect").primary().label("开启保护").on_click(cx.listener(|this, _, _, cx| this.protect(cx)))).child(Button::new("restore").label("恢复所选窗口").on_click(cx.listener(|this, _, _, cx| this.restore_selected(cx)))))
            .child(div().rounded_md().bg(theme.secondary).p_3().text_sm().text_color(theme.muted_foreground).child("普通窗口建议启用系统保护；自绘标题栏启用鼠标遮罩。遮罩会拦截关闭按钮区域的单击和双击。"))
            .child(div().text_sm().text_color(theme.muted_foreground).child("关闭或最小化主界面后，保护继续在后台运行。点击托盘图标返回，使用“退出”恢复保护并结束程序。"))
            .child(div().text_sm().text_color(theme.muted_foreground).child(self.status.clone()))
    }
}

pub fn run() {
    gpui_kit::application().run(|cx| {
        gpui_kit::init(cx);
        crate::protection::register_overlay_class()
            .expect("failed to register shield window class");
        let background = std::env::args().any(|arg| arg == "--background");
        let options = WindowOptions {
            show: !background,
            focus: !background,
            ..WindowOptions::default()
        };
        let (handle, view) = gpui_kit::open_window(options, cx, |window, cx| {
            cx.new(|cx| KeepWindow::new(window, cx))
        })
        .expect("failed to open KeepWindow");
        if background && view.read(cx).tray.is_none() {
            handle
                .update(cx, |_, window, _| {
                    if let Ok(native) = HasWindowHandle::window_handle(window)
                        && let RawWindowHandle::Win32(native) = native.as_raw()
                    {
                        unsafe {
                            let _ = windows::Win32::UI::WindowsAndMessaging::ShowWindow(
                                HWND(native.hwnd.get() as *mut _),
                                windows::Win32::UI::WindowsAndMessaging::SW_SHOW,
                            );
                        }
                    }
                })
                .ok();
        }
    });
}
