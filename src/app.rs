use std::time::Duration;

use gpui_kit::{
    AppContext as _, Bounds, Context, Entity, IntoElement, ParentElement as _, Render,
    SharedString, Styled as _, Window, WindowBounds, WindowOptions,
    assets::IconName,
    base::StyledExt,
    component::{
        ActiveTheme, Disableable as _, Icon, Sizable as _,
        button::Button,
        button::ButtonVariants,
        checkbox::Checkbox,
        input::{Input, InputEvent, InputState, NumberInput},
        table::{DataTable, TableEvent, TableState},
    },
    div, px, size,
};
use raw_window_handle::{HasWindowHandle, RawWindowHandle};
use windows::Win32::Foundation::HWND;

use crate::{
    protection::{Options, Protection},
    tray::{Action as TrayAction, Tray},
    ui::{self, WindowRow, WindowTable},
    window::{self, WindowInfo},
};

pub struct KeepWindow {
    windows: Vec<WindowInfo>,
    protections: Vec<Protection>,
    selected: Option<WindowInfo>,
    table: Entity<TableState<WindowTable>>,
    search: Entity<InputState>,
    drafts: Vec<(WindowInfo, Draft)>,
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
    _subscriptions: Vec<gpui_kit::Subscription>,
}

#[derive(Clone)]
struct Draft {
    system_menu: bool,
    mouse_shield: bool,
    numbers: [String; 4],
}

impl From<Options> for Draft {
    fn from(options: Options) -> Self {
        Self {
            system_menu: options.system_menu,
            mouse_shield: options.mouse_shield,
            numbers: [
                options.width,
                options.height,
                options.right_offset,
                options.top_offset,
            ]
            .map(|value| value.to_string()),
        }
    }
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
                .step(1.)
                .min(1.)
                .max(512.)
        });
        let height = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder("高度")
                .default_value("32")
                .step(1.)
                .min(1.)
                .max(512.)
        });
        let right_offset = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder("右侧内缩")
                .default_value("0")
                .step(1.)
                .min(-512.)
                .max(512.)
        });
        let top_offset = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder("顶部下移")
                .default_value("0")
                .step(1.)
                .min(-512.)
                .max(512.)
        });
        let search =
            cx.new(|cx| InputState::new(window, cx).placeholder("搜索窗口名称、PID 或句柄"));
        let table = cx.new(|cx| {
            TableState::new(WindowTable::new(), window, cx)
                .col_selectable(false)
                .col_movable(false)
                .sortable(false)
                .loop_selection(false)
        });
        let mut subscriptions = vec![
            cx.subscribe_in(
                &table,
                window,
                |this, table, event, window, cx| match event {
                    TableEvent::SelectRow(index) => {
                        if table.read(cx).selected_row() != Some(*index) {
                            return;
                        }
                        let target = table
                            .read(cx)
                            .delegate()
                            .rows
                            .get(*index)
                            .map(|row| row.info.clone());
                        if let Some(target) = target {
                            table.update(cx, |state, cx| {
                                state.delegate_mut().selected_row = Some(*index);
                                cx.notify();
                            });
                            this.select_window(target, window, cx);
                        }
                    }
                    TableEvent::ClearSelection => {
                        if table.read(cx).selected_row().is_some() {
                            return;
                        }
                        table.update(cx, |state, cx| {
                            state.delegate_mut().selected_row = None;
                            cx.notify();
                        });
                        this.save_draft(cx);
                        this.selected = None;
                        cx.notify();
                    }
                    _ => {}
                },
            ),
            cx.subscribe_in(&search, window, |this, _, event, _, cx| {
                if matches!(event, InputEvent::Change) {
                    this.rebuild_table(cx);
                    cx.notify();
                }
            }),
        ];
        for input in [&width, &height, &right_offset, &top_offset] {
            subscriptions.push(cx.subscribe(input, |_, _, event, cx| {
                if matches!(event, InputEvent::Change) {
                    cx.notify();
                }
            }));
        }
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
            table,
            search,
            drafts: Vec::new(),
            status: "选择一个窗口，设置保护方式后开启保护。".into(),
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
            _subscriptions: subscriptions,
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
        self.save_draft(cx);
        match window::enumerate() {
            Ok(windows) => {
                self.selected = window::retain_selection(&windows, self.selected.as_ref(), "");
                self.drafts
                    .retain(|(target, _)| windows.iter().any(|item| item.same_identity(target)));
                self.windows = windows;
                self.rebuild_table(cx);
                self.status = "窗口列表已更新。".into();
            }
            Err(error) => self.status = format!("刷新失败：{error}").into(),
        }
        cx.notify();
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
        self.selected.clone()
    }

    fn protection_for(&self, target: &WindowInfo) -> Option<&Protection> {
        self.protections
            .iter()
            .find(|p| p.target.same_identity(target))
    }

    fn save_draft(&mut self, cx: &gpui_kit::App) {
        let Some(target) = self.selected.clone() else {
            return;
        };
        if self.protection_for(&target).is_some() {
            return;
        }
        let draft = Draft {
            system_menu: self.system_menu,
            mouse_shield: self.mouse_shield,
            numbers: [
                &self.width,
                &self.height,
                &self.right_offset,
                &self.top_offset,
            ]
            .map(|input| input.read(cx).value().to_string()),
        };
        if let Some((_, existing)) = self
            .drafts
            .iter_mut()
            .find(|(item, _)| item.same_identity(&target))
        {
            *existing = draft;
        } else {
            self.drafts.push((target, draft));
        }
    }

    fn select_window(&mut self, target: WindowInfo, window: &mut Window, cx: &mut Context<Self>) {
        if self
            .selected
            .as_ref()
            .is_some_and(|item| item.same_identity(&target))
        {
            self.selected = Some(target);
            return;
        }
        self.save_draft(cx);
        let draft = self
            .protection_for(&target)
            .map(|p| Draft::from(p.options))
            .or_else(|| {
                self.drafts
                    .iter()
                    .find(|(item, _)| item.same_identity(&target))
                    .map(|(_, draft)| draft.clone())
            })
            .unwrap_or_else(|| Options::default().into());
        self.system_menu = draft.system_menu;
        self.mouse_shield = draft.mouse_shield;
        for (input, value) in [
            &self.width,
            &self.height,
            &self.right_offset,
            &self.top_offset,
        ]
        .into_iter()
        .zip(draft.numbers)
        {
            input.update(cx, |state, cx| state.set_value(value, window, cx));
        }
        self.selected = Some(target);
        cx.notify();
    }

    fn rebuild_table(&mut self, cx: &mut Context<Self>) {
        let query = self.search.read(cx).value().trim().to_lowercase();
        if window::retain_selection(&self.windows, self.selected.as_ref(), &query).is_none() {
            self.save_draft(cx);
            self.selected = None;
        }
        let rows: Vec<_> = self
            .windows
            .iter()
            .filter(|item| item.matches_query(&query))
            .map(|item| WindowRow {
                info: item.clone(),
                protection: self.protection_for(item).map(|p| {
                    match (p.options.system_menu, p.options.mouse_shield) {
                        (true, true) => "系统关闭保护 + 鼠标遮罩",
                        (true, false) => "系统关闭保护",
                        _ => "鼠标遮罩",
                    }
                }),
            })
            .collect();
        let selected_row = self
            .selected
            .as_ref()
            .and_then(|selected| rows.iter().position(|row| row.info.same_identity(selected)));
        if selected_row.is_none() {
            self.save_draft(cx);
            self.selected = None;
        }
        self.table.update(cx, |table, cx| {
            table.delegate_mut().rows = rows;
            table.delegate_mut().filtered = !query.is_empty();
            table.delegate_mut().selected_row = selected_row;
            match selected_row {
                Some(index) if table.selected_row() != Some(index) => {
                    table.set_selected_row(index, cx)
                }
                None if table.selected_row().is_some() => table.clear_selection(cx),
                _ => {}
            }
            cx.notify();
        });
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
            cx.notify();
            return;
        };
        if self.protection_for(&target).is_some() {
            self.status = "所选窗口已受保护，请先恢复后再修改参数。".into();
            cx.notify();
            return;
        }
        self.save_draft(cx);
        match self.options(cx) {
            Ok(options) => match Protection::start(target.clone(), options) {
                Ok(p) => {
                    self.protections.push(p);
                    self.status = format!("已保护“{}”。", target.display_title()).into();
                }
                Err(e) => self.status = e.into(),
            },
            Err(e) => self.status = e,
        }
        self.rebuild_table(cx);
        cx.notify();
    }

    fn restore_selected(&mut self, cx: &mut Context<Self>) {
        let Some(target) = self.selected_window() else {
            self.status = "请先选择要恢复的窗口。".into();
            cx.notify();
            return;
        };
        let Some(index) = self
            .protections
            .iter()
            .position(|p| p.target.same_identity(&target))
        else {
            self.status = "所选窗口尚未开启保护。".into();
            cx.notify();
            return;
        };
        match self.protections[index].restore() {
            Ok(()) => {
                self.protections.remove(index);
                self.status = format!("已恢复“{}”。", target.display_title()).into();
            }
            Err(e) => self.status = e.into(),
        }
        self.rebuild_table(cx);
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
        self.rebuild_table(cx);
        cx.notify();
    }
}

impl Render for KeepWindow {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        self.table.update(cx, |table, cx| {
            if table
                .delegate_mut()
                .fit_width(window.bounds().size.width - px(50.))
            {
                table.refresh(cx);
            }
        });
        let theme = cx.theme();
        let compact = window.bounds().size.height < px(720.);
        let narrow = window.bounds().size.width < px(840.);
        let selected = self.selected.as_ref();
        let protected = selected.is_some_and(|target| self.protection_for(target).is_some());
        let editable = selected.is_some() && !protected;
        let parameters_enabled = editable && self.mouse_shield;
        let valid = self.options(cx).is_ok();
        let can_protect = editable && (self.system_menu || self.mouse_shield) && valid;
        let count = self.table.read(cx).delegate().rows.len();
        let filtering = !self.search.read(cx).value().trim().is_empty();
        let count_label = if filtering {
            format!("匹配 {count} / 共 {} 个窗口", self.windows.len())
        } else {
            format!("共找到 {} 个窗口", self.windows.len())
        };
        let config_hint: SharedString = if protected {
            "当前展示生效参数；恢复后可修改。".into()
        } else if editable && !self.system_menu && !self.mouse_shield {
            "请选择至少一种保护方式。".into()
        } else if parameters_enabled && !valid {
            self.options(cx).unwrap_err()
        } else {
            "参数仅应用于当前所选窗口。".into()
        };
        let fields = [
            ("宽度", &self.width, "1–512"),
            ("高度", &self.height, "1–512"),
            ("右侧内缩", &self.right_offset, "−512–512"),
            ("顶部下移", &self.top_offset, "−512–512"),
        ]
        .into_iter()
        .map(|(label, state, range)| {
            div()
                .flex_1()
                .min_w_0()
                .v_flex()
                .gap_1p5()
                .child(
                    div()
                        .flex()
                        .justify_between()
                        .text_xs()
                        .child(label)
                        .child(div().text_color(theme.muted_foreground).child(range)),
                )
                .child(NumberInput::new(state).disabled(!parameters_enabled))
        });
        div().size_full().v_flex().bg(theme.background).text_color(theme.foreground).font_family(theme.font_family.clone())
            .child(
                div().flex().items_center().justify_between().gap_4().px_6().py_4().flex_shrink_0()
                    .child(div().flex().items_center().gap_3()
                        .child(div().size(px(38.)).rounded(px(12.)).bg(theme.primary).text_color(theme.primary_foreground).flex().items_center().justify_center().child(Icon::new(IconName::ShieldCheck).size(px(22.))))
                        .child(div().font_family("Segoe UI Variable Display").text_xl().font_semibold().child("KeepWindow")))
                    .child(div().flex().items_center().gap_1p5()
                        .child(Button::new("hint").ghost().small().icon(IconName::Lightbulb).accessibility_label("保护使用提示").tooltip("标准标题栏可使用系统关闭保护；自绘标题栏可开启鼠标遮罩。收起或关闭主界面后，保护继续运行。"))
                        .child(Button::new("refresh").ghost().small().icon(IconName::RefreshCw).label("刷新").tooltip("重新读取窗口列表").on_click(cx.listener(|this, _, _, cx| this.refresh(cx))))
                        .child(Button::new("restore-all").ghost().small().icon(IconName::RotateCcw).label("恢复全部").disabled(self.protections.is_empty()).on_click(cx.listener(|this, _, _, cx| this.restore_all(cx))))
                        .child(Button::new("hide").ghost().small().icon(IconName::Minimize2).label(if narrow { "收起" } else { "收起到托盘" }).tooltip("隐藏主界面，保护继续在后台运行").on_click(cx.listener(|this, _, _, cx| this.minimize_to_tray(cx))))
                        .child(div().w(px(1.)).h_5().mx_1().bg(theme.border))
                        .child(Button::new("exit").ghost().small().icon(IconName::LogOut).label("退出").tooltip("恢复全部窗口并退出 KeepWindow").on_click(cx.listener(|this, _, _, cx| this.exit(cx)))))
            )
            .child(
                div().flex_1().min_h_0().v_flex().gap_4().px_6().pb_4()
                    .child(
                        div().flex_1().min_h(px(110.)).v_flex().rounded(px(12.)).border_1().border_color(theme.border).bg(theme.table).overflow_hidden()
                            .child(div().flex().items_center().justify_between().gap_4().px_4().py_3().flex_shrink_0()
                                .child(div().flex().items_center().gap_2().child(div().text_sm().font_semibold().child("窗口列表")).child(div().text_xs().text_color(theme.muted_foreground).child(format!("{count} 个"))))
                                .child(div().w(px(if narrow { 280. } else { 320. })).child(Input::new(&self.search).small().prefix(Icon::new(IconName::Search).small()).cleanable(true))))
                            .child(div().flex_1().min_h_0().child(DataTable::new(&self.table).bordered(false).with_size(px(if compact { 32. } else { 36. })).header_row_height(px(34.)))))
                    .child(
                        div().v_flex().gap_3().p_4().flex_shrink_0().rounded(px(12.)).border_1().border_color(theme.border).bg(theme.table)
                            .child(div().flex().items_center().gap_6()
                                .child(Checkbox::new("system-menu").label("禁用系统关闭").checked(self.system_menu).disabled(!editable).on_change(cx.listener(|this, v, _, cx| { this.system_menu = *v; cx.notify(); })))
                                .child(Checkbox::new("mouse-shield").label("鼠标遮罩").checked(self.mouse_shield).disabled(!editable).on_change(cx.listener(|this, v, _, cx| { this.mouse_shield = *v; cx.notify(); })))
                                .child(div().text_xs().text_color(theme.muted_foreground).child(if self.mouse_shield { "拦截关闭按钮区域的鼠标点击" } else { "适用于标准标题栏" })))
                            .child(div().v_flex().gap_2()
                                .child(div().flex().items_center().gap_2().text_xs().child("遮罩尺寸与位置").child(div().text_color(theme.muted_foreground).child("DIP · 随目标窗口缩放")))
                                .child(div().flex().gap_3().children(fields)))
                            .child(div().flex().items_center().justify_between().gap_4()
                                .child(div().flex_1().min_w_0().text_xs().text_color(if editable && (!valid || (!self.system_menu && !self.mouse_shield)) { theme.danger } else { theme.muted_foreground }).child(config_hint))
                                .child(div().flex().gap_2()
                                    .child(Button::new("restore").outline().icon(IconName::RotateCcw).label("恢复所选窗口").disabled(!protected).on_click(cx.listener(|this, _, _, cx| this.restore_selected(cx))))
                                    .child(Button::new("protect").primary().icon(IconName::ShieldCheck).label(if protected { "已开启保护" } else { "开启保护" }).disabled(!can_protect).on_click(cx.listener(|this, _, _, cx| this.protect(cx)))))))
            )
            .child(
                div().v_flex().gap_1().px_6().py_2().flex_shrink_0().border_t_1().border_color(theme.border).bg(theme.table)
                    .child(div().flex().items_center().justify_between().text_xs()
                        .child(div().flex().items_center().gap_3().text_color(theme.muted_foreground).child(count_label).child(div().w(px(1.)).h_3().bg(theme.border)).child(format!("正在保护 {} 个窗口", self.protections.len())))
                        .child(div().flex_1().min_w_0().text_right().truncate().text_color(theme.muted_foreground).child(self.status.clone()))))
    }
}

pub fn run() {
    gpui_kit::application()
        .with_assets(gpui_kit::assets::AllAssets)
        .run(|cx| {
            gpui_kit::init(cx);
            ui::init_theme(cx);
            crate::protection::register_overlay_class()
                .expect("failed to register shield window class");
            let background = std::env::args().any(|arg| arg == "--background");
            let options = WindowOptions {
                show: !background,
                focus: !background,
                window_bounds: Some(WindowBounds::Windowed(Bounds::centered(
                    None,
                    size(px(920.), px(780.)),
                    cx,
                ))),
                window_min_size: Some(size(px(740.), px(620.))),
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
