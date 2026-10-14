use std::{
    collections::HashMap,
    time::{Duration, Instant},
};

use gpui_kit::{
    AppContext as _, Bounds, Context, Entity, InteractiveElement as _, IntoElement,
    ParentElement as _, Render, SharedString, StatefulInteractiveElement as _, Styled as _, Window,
    WindowBounds, WindowOptions,
    assets::IconName,
    base::StyledExt,
    component::{
        ActiveTheme, Disableable as _, Icon, Sizable as _,
        button::Button,
        button::ButtonVariants,
        checkbox::Checkbox,
        input::{Input, InputEvent, InputState, NumberInput},
        menu::{DropdownMenu as _, PopupMenuItem},
        table::{DataTable, TableEvent, TableState},
        tooltip::Tooltip,
    },
    div,
    prelude::FluentBuilder as _,
    px, size,
};
use raw_window_handle::{HasWindowHandle, RawWindowHandle};
use windows::Win32::Foundation::HWND;

use crate::{
    calibration::{Calibration, Outcome},
    config::{Config, Rule, Store},
    picker::Picker,
    protection::{Options, Protection},
    recovery::Recovery,
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
    opacity: Entity<InputState>,
    title_contains: Entity<InputState>,
    automatic: bool,
    config: Config,
    store: Store,
    recovery: Recovery,
    startup_enabled: bool,
    config_error: Option<String>,
    rules_open: bool,
    rule_settings_open: bool,
    editing_rule: Option<usize>,
    suppressed: Vec<WindowInfo>,
    attempts: Vec<(WindowInfo, Rule, Option<String>)>,
    paths: HashMap<u32, (Option<u64>, Option<String>)>,
    last_scan: Instant,
    exiting: bool,
    calibration: Option<Calibration>,
    picker: Option<Picker>,
    tray: Option<Tray>,
    _refresh_task: gpui_kit::Task<()>,
    _quit_subscription: gpui_kit::Subscription,
    _subscriptions: Vec<gpui_kit::Subscription>,
}

#[derive(Clone)]
struct Draft {
    system_menu: bool,
    mouse_shield: bool,
    numbers: [String; 5],
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
                options.opacity,
            ]
            .map(|value| value.to_string()),
        }
    }
}

impl KeepWindow {
    fn new(
        window: &mut Window,
        cx: &mut Context<Self>,
        recovery: Recovery,
        recovery_message: Option<String>,
    ) -> Self {
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
        let opacity = cx.new(|cx| {
            InputState::new(window, cx)
                .default_value("28")
                .step(1.)
                .min(5.)
                .max(80.)
        });
        let title_contains =
            cx.new(|cx| InputState::new(window, cx).placeholder("标题关键词（可选）"));
        let (store, config, config_error) = Store::load();
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
                    _ if this.picker.is_some() => {}
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
        for input in [
            &width,
            &height,
            &right_offset,
            &top_offset,
            &opacity,
            &title_contains,
        ] {
            subscriptions.push(cx.subscribe(input, |_, _, event, cx| {
                if matches!(event, InputEvent::Change) {
                    cx.notify();
                }
            }));
        }
        let quit_subscription = cx.on_app_quit(|this, _cx| {
            this.picker.take();
            for protection in &mut this.protections {
                let _ = protection.restore();
            }
            this.protections.clear();
            let _ = this.recovery.finish();
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
            opacity,
            title_contains,
            automatic: false,
            config,
            store,
            recovery,
            startup_enabled: false,
            config_error,
            rules_open: false,
            rule_settings_open: false,
            editing_rule: None,
            suppressed: Vec::new(),
            attempts: Vec::new(),
            paths: HashMap::new(),
            last_scan: Instant::now(),
            exiting: false,
            calibration: None,
            picker: None,
            tray: None,
            _refresh_task: cx.spawn_in(window, async move |this, cx| {
                loop {
                    cx.background_executor()
                        .timer(Duration::from_millis(100))
                        .await;
                    if this
                        .update_in(cx, |this, window, cx| {
                            this.tick_picker(window, cx);
                            this.tick_calibration(window, cx);
                            this.tick(cx);
                            if this.last_scan.elapsed() >= Duration::from_secs(1) {
                                this.scan(cx);
                            }
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
        match tray {
            Ok(tray) => {
                this.tray = Some(tray);
            }
            Err(error) => this.status = error.into(),
        }
        this.scan(cx);
        let mut messages = Vec::new();
        if let Some(message) = recovery_message {
            messages.push(message);
        }
        match crate::startup::enabled() {
            Ok(enabled) => this.startup_enabled = enabled,
            Err(error) => messages.push(error),
        }
        if let Some(error) = &this.config_error {
            messages.push(error.clone());
        }
        if !messages.is_empty() {
            this.status = messages.join("；").into();
            if let Some(tray) = &this.tray {
                tray.notify(&this.status);
            }
        }
        this
    }

    fn refresh(&mut self, cx: &mut Context<Self>) {
        self.scan(cx);
        cx.notify();
    }

    fn scan(&mut self, cx: &mut Context<Self>) {
        self.last_scan = Instant::now();
        self.save_draft(cx);
        match window::enumerate() {
            Ok(mut windows) => {
                for protection in &self.protections {
                    if protection.alive()
                        && !windows
                            .iter()
                            .any(|item| item.same_identity(&protection.target))
                    {
                        windows.push(
                            WindowInfo::inspect(protection.target.hwnd)
                                .unwrap_or_else(|| protection.target.clone()),
                        );
                    }
                }
                self.suppressed.retain(|target| target.is_same_window());
                self.attempts
                    .retain(|(target, _, _)| target.is_same_window());
                self.paths
                    .retain(|pid, _| windows.iter().any(|item| item.pid == *pid));
                for target in &windows {
                    let creation = window::process_creation(target.pid);
                    if self
                        .paths
                        .get(&target.pid)
                        .is_none_or(|(previous, _)| creation.is_none() || *previous != creation)
                    {
                        self.paths
                            .insert(target.pid, (creation, window::process_path(target.pid)));
                    }
                }
                self.selected = window::retain_selection(&windows, self.selected.as_ref(), "");
                self.drafts
                    .retain(|(target, _)| windows.iter().any(|item| item.same_identity(target)));
                self.windows = windows;
                if !self.exiting {
                    self.auto_protect();
                }
                self.rebuild_table(cx);
            }
            Err(error) => self.status = format!("刷新失败：{error}").into(),
        }
        cx.notify();
    }

    fn tick(&mut self, cx: &mut Context<Self>) {
        let mut ended = Vec::new();
        self.protections.retain_mut(|protection| {
            if protection.recovery_failed {
                return protection.alive();
            }
            match protection.tick() {
                Ok(()) => true,
                Err(error) => match protection.restore() {
                    Ok(()) => {
                        ended.push(error);
                        self.suppressed.push(protection.target.clone());
                        false
                    }
                    Err(e) => {
                        ended.push(format!("{error}；恢复失败：{e}"));
                        true
                    }
                },
            }
        });
        if !ended.is_empty() {
            self.refresh(cx);
            self.status = ended.join("；").into();
            if let Some(tray) = &self.tray {
                tray.notify(&self.status);
            }
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

    fn exit(&mut self, cx: &mut Context<Self>) {
        self.exiting = true;
        self.picker.take();
        self.calibration.take();
        for protection in &mut self.protections {
            protection.set_calibration(None);
        }
        self.restore_all(cx);
        if self.protections.is_empty() {
            if let Err(error) = self.recovery.finish() {
                self.status = error.into();
                if let Some(tray) = &self.tray {
                    tray.notify(&self.status);
                }
            }
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
                &self.opacity,
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
            .or_else(|| {
                self.path_for(&target)
                    .and_then(|path| self.config.matching_rule(path, &target, false))
                    .map(|(_, rule)| Draft::from(rule.options))
            })
            .unwrap_or_else(|| Options::default().into());
        self.system_menu = draft.system_menu;
        self.mouse_shield = draft.mouse_shield;
        for (input, value) in [
            &self.width,
            &self.height,
            &self.right_offset,
            &self.top_offset,
            &self.opacity,
        ]
        .into_iter()
        .zip(draft.numbers)
        {
            input.update(cx, |state, cx| state.set_value(value, window, cx));
        }
        self.selected = Some(target);
        self.editing_rule = None;
        let matched = self
            .selected
            .as_ref()
            .and_then(|target| {
                self.path_for(target)
                    .and_then(|path| self.config.matching_rule(path, target, false))
            })
            .map(|(index, rule)| (index, rule.clone()));
        if let Some((index, rule)) = matched {
            self.editing_rule = Some(index);
            self.automatic = rule.automatic;
            self.title_contains.update(cx, |state, cx| {
                state.set_value(rule.title_contains, window, cx)
            });
        } else {
            self.automatic = false;
            self.title_contains
                .update(cx, |state, cx| state.set_value("", window, cx));
        }
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
                protection: self
                    .protection_for(item)
                    .map(|p| {
                        let status = p.status();
                        (status.label().into(), status.description())
                    })
                    .or_else(|| {
                        self.attempts
                            .iter()
                            .find(|(target, _, error)| {
                                target.same_identity(item) && error.is_some()
                            })
                            .map(|(_, _, error)| ("保护失败".into(), error.clone().unwrap()))
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
        for (field, input, name, min, max) in [
            (&mut options.width, &self.width, "遮罩宽度", 1, 512),
            (&mut options.height, &self.height, "遮罩高度", 1, 512),
            (
                &mut options.right_offset,
                &self.right_offset,
                "右侧内缩",
                -512,
                512,
            ),
            (
                &mut options.top_offset,
                &self.top_offset,
                "顶部下移",
                -512,
                512,
            ),
            (&mut options.opacity, &self.opacity, "透明度", 5, 80),
        ] {
            match number(input, name, min, max) {
                Ok(value) => *field = value,
                Err(error) if self.mouse_shield => return Err(error.into()),
                _ => {}
            }
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
            return;
        }
        self.suppressed.retain(|item| !item.same_identity(&target));
        self.attempts
            .retain(|(item, _, _)| !item.same_identity(&target));
        self.save_draft(cx);
        match self.options(cx) {
            Ok(options) => {
                match Protection::start_logged(target.clone(), options, self.recovery.clone()) {
                    Ok(p) => {
                        self.protections.push(p);
                        self.status = format!("已保护“{}”。", target.display_title()).into();
                    }
                    Err(e) => self.status = e.into(),
                }
            }
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
        self.suppressed.push(target.clone());
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
        self.suppressed
            .extend(self.protections.iter().map(|p| p.target.clone()));
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

    fn path_for(&self, target: &WindowInfo) -> Option<&str> {
        self.paths
            .get(&target.pid)
            .and_then(|(_, path)| path.as_deref())
    }

    fn auto_protect(&mut self) {
        if self.picker.is_some() {
            return;
        }
        for target in self.windows.clone() {
            if self.protection_for(&target).is_some()
                || self
                    .suppressed
                    .iter()
                    .any(|item| item.same_identity(&target))
            {
                continue;
            }
            let rule = self
                .path_for(&target)
                .and_then(|path| self.config.matching_rule(path, &target, true))
                .map(|(_, rule)| rule.clone());
            let Some(rule) = rule else {
                continue;
            };
            if self
                .attempts
                .iter()
                .any(|(item, previous, _)| item.same_identity(&target) && previous == &rule)
            {
                continue;
            }
            self.attempts
                .retain(|(item, _, _)| !item.same_identity(&target));
            match Protection::start_logged(target.clone(), rule.options, self.recovery.clone()) {
                Ok(protection) => {
                    self.protections.push(protection);
                    self.attempts.push((target, rule, None));
                }
                Err(error) => {
                    self.status =
                        format!("自动保护“{}”失败：{error}", target.display_title()).into();
                    if let Some(tray) = &self.tray {
                        tray.notify(&self.status);
                    }
                    self.attempts.push((target, rule, Some(error)));
                }
            }
        }
    }

    fn set_fields(&mut self, options: Options, window: &mut Window, cx: &mut Context<Self>) {
        self.system_menu = options.system_menu;
        self.mouse_shield = options.mouse_shield;
        for (input, value) in [
            &self.width,
            &self.height,
            &self.right_offset,
            &self.top_offset,
            &self.opacity,
        ]
        .into_iter()
        .zip(Draft::from(options).numbers)
        {
            input.update(cx, |state, cx| state.set_value(value, window, cx));
        }
    }

    fn save_rule(&mut self, cx: &mut Context<Self>) {
        let rule = (|| -> Result<Rule, SharedString> {
            let editing = self
                .editing_rule
                .and_then(|index| self.config.rules.get(index));
            let target = self.selected.as_ref();
            let path = editing
                .map(|rule| rule.path.as_str())
                .or_else(|| target.and_then(|target| self.path_for(target)))
                .ok_or("无法获取程序路径，不能保存规则。")?;
            let class = editing
                .map(|rule| rule.class.clone())
                .or_else(|| target.map(|target| target.class.clone()))
                .ok_or("请先选择窗口。")?;
            Ok(Rule {
                path: path.into(),
                class,
                title_contains: self.title_contains.read(cx).value().trim().to_string(),
                automatic: self.automatic,
                options: self.options(cx)?,
            })
        })();
        match rule {
            Ok(rule) => {
                let mut config = self.config.clone();
                config.upsert(rule, self.editing_rule);
                self.commit_config(config);
                self.editing_rule = None;
            }
            Err(error) => self.status = error,
        }
        cx.notify();
    }

    fn commit_config(&mut self, config: Config) {
        match self.store.save(&config) {
            Ok(()) => {
                self.attempts
                    .retain(|(_, rule, _)| config.rules.iter().any(|current| current == rule));
                self.config = config;
                self.status = "应用规则已保存。".into();
            }
            Err(error) => self.status = error.into(),
        }
    }

    fn edit_rule(&mut self, index: usize, window: &mut Window, cx: &mut Context<Self>) {
        self.save_draft(cx);
        self.selected = None;
        self.rebuild_table(cx);
        let rule = self.config.rules[index].clone();
        self.set_fields(rule.options, window, cx);
        self.title_contains.update(cx, |state, cx| {
            state.set_value(rule.title_contains, window, cx)
        });
        self.automatic = rule.automatic;
        self.editing_rule = Some(index);
        self.rule_settings_open = true;
        self.status = "正在编辑已保存规则，点击保存完成修改。".into();
        cx.notify();
    }

    fn apply_settings(&mut self, cx: &mut Context<Self>) {
        let result = self.options(cx).and_then(|options| {
            let target = self.selected.clone().ok_or("请先选择窗口。")?;
            let protection = self
                .protections
                .iter_mut()
                .find(|p| p.target.same_identity(&target))
                .ok_or("窗口未保护。")?;
            protection
                .update_options(options)
                .map_err(SharedString::from)
        });
        self.status = match result {
            Ok(()) => "保护设置已更新。".into(),
            Err(error) => error,
        };
        self.rebuild_table(cx);
        cx.notify();
    }

    fn start_calibration(&mut self, cx: &mut Context<Self>) {
        if self.picker.is_some() {
            return;
        }
        let result = self.options(cx).and_then(|options| {
            let target = self.selected.clone().ok_or("请先选择窗口。")?;
            if self
                .protection_for(&target)
                .is_some_and(|p| p.recovery_failed)
            {
                return Err("请先重试恢复。".into());
            }
            Calibration::new(target, options).map_err(SharedString::from)
        });
        match result {
            Ok(mut calibration) => {
                if let Some(protection) = self
                    .protections
                    .iter_mut()
                    .find(|p| p.target.same_identity(&calibration.target))
                {
                    calibration.previous_protection = Some(protection.options);
                    protection.set_calibration(Some(calibration.frame()));
                }
                self.calibration = Some(calibration);
                self.status = "拖动遮罩移动，拖动右下角缩放；Enter 保存，Esc 取消。".into();
            }
            Err(error) => self.status = error,
        }
        cx.notify();
    }

    fn start_picker(&mut self, cx: &mut Context<Self>) {
        if self.picker.is_some() || self.calibration.is_some() {
            return;
        }
        match Picker::new() {
            Ok(picker) => {
                self.picker = Some(picker);
                for protection in &mut self.protections {
                    protection.set_calibration(Some(HWND::default()));
                }
                self.status = "移动准星到目标窗口，单击选择；Esc 取消。".into();
            }
            Err(error) => self.status = error.into(),
        }
        cx.notify();
    }

    fn tick_picker(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(picker) = &mut self.picker else {
            return;
        };
        let outcome = picker.tick();
        if matches!(outcome, Ok(None)) {
            return;
        }
        self.picker.take();
        for protection in &mut self.protections {
            protection.set_calibration(None);
        }
        match outcome {
            Ok(Some(Some(target))) => {
                // Suppress automatic rules for this target until the user explicitly protects it.
                self.suppressed.push(target.clone());
                self.search
                    .update(cx, |state, cx| state.set_value("", window, cx));
                self.rules_open = false;
                self.scan(cx);
                if target.is_same_window() {
                    if !self.windows.iter().any(|item| item.same_identity(&target)) {
                        self.windows.push(target.clone());
                    }
                    self.select_window(target.clone(), window, cx);
                    if self.editing_rule.is_none() {
                        self.title_contains.update(cx, |state, cx| {
                            state.set_value(target.title.clone(), window, cx)
                        });
                    }
                    self.rebuild_table(cx);
                    self.status =
                        format!("已选择“{}”，请确认参数后开启保护。", target.display_title())
                            .into();
                } else {
                    self.status = "目标窗口已关闭，请重新选择。".into();
                }
            }
            Ok(_) => self.status = "已取消窗口选择。".into(),
            Err(error) => self.status = error.into(),
        }
        self.rebuild_table(cx);
        cx.notify();
    }

    fn toggle_startup(&mut self, cx: &mut Context<Self>) {
        let enabled = !self.startup_enabled;
        let result = crate::startup::set_enabled(enabled).and_then(|previous| {
            let mut config = self.config.clone();
            config.startup_enabled = enabled;
            if let Err(error) = self.store.save(&config) {
                return Err(match crate::startup::restore(previous.as_deref()) {
                    Ok(()) => error,
                    Err(rollback) => format!("{error}；恢复开机启动设置失败：{rollback}"),
                });
            }
            self.config = config;
            self.startup_enabled = enabled;
            Ok(())
        });
        self.status = match result {
            Ok(()) if enabled => "已开启当前用户开机启动，登录后在后台运行。".into(),
            Ok(()) => "已关闭开机启动。".into(),
            Err(error) => {
                if let Ok(enabled) = crate::startup::enabled() {
                    self.startup_enabled = enabled;
                }
                error.into()
            }
        };
        cx.notify();
    }

    fn tick_calibration(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(calibration) = &mut self.calibration else {
            return;
        };
        let mut outcome = calibration.tick();
        if let Some(protection) = self
            .protections
            .iter_mut()
            .find(|p| p.target.same_identity(&calibration.target))
        {
            let mut options = protection.options;
            options.width = calibration.current.width;
            options.height = calibration.current.height;
            options.right_offset = calibration.current.right_offset;
            options.top_offset = calibration.current.top_offset;
            if let Err(error) = protection.update_options(options) {
                outcome = Err(error);
            }
        }
        if matches!(outcome, Ok(None)) {
            return;
        }
        let calibration = self.calibration.take().unwrap();
        let saved = matches!(outcome, Ok(Some(Outcome::Save)));
        let options = if saved {
            calibration.current
        } else {
            calibration.original
        };
        let mut error = outcome.as_ref().err().cloned();
        if let Some(protection) = self
            .protections
            .iter_mut()
            .find(|p| p.target.same_identity(&calibration.target))
        {
            protection.set_calibration(None);
            let restored = if saved {
                options
            } else {
                calibration.previous_protection.unwrap_or(options)
            };
            if protection.alive()
                && let Err(e) = protection.update_options(restored)
            {
                error = Some(e);
            }
        }
        if self
            .selected
            .as_ref()
            .is_some_and(|target| target.same_identity(&calibration.target))
        {
            self.set_fields(options, window, cx);
        }
        if saved
            && error.is_none()
            && let Some(path) = self.path_for(&calibration.target)
            && let Some((index, _)) = self.config.matching_rule(path, &calibration.target, false)
        {
            let mut config = self.config.clone();
            config.rules[index].options = options;
            match self.store.save(&config) {
                Ok(()) => self.config = config,
                Err(e) => error = Some(e),
            }
        }
        self.status = match error {
            Some(error) => error.into(),
            _ if saved => "遮罩校准已保存。".into(),
            _ => "已取消校准并恢复原位置。".into(),
        };
        self.rebuild_table(cx);
        cx.notify();
    }
}

impl Render for KeepWindow {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        self.table.update(cx, |table, cx| {
            table.delegate_mut().sync_icons(&self.windows, window, cx);
            if table
                .delegate_mut()
                .fit_width(window.bounds().size.width - px(34.))
            {
                table.refresh(cx);
            }
        });
        let theme = cx.theme();
        let selected = self.selected.as_ref();
        let protected = selected.is_some_and(|target| self.protection_for(target).is_some());
        let recovery_failed = selected.is_some_and(|target| {
            self.protection_for(target)
                .is_some_and(|p| p.recovery_failed)
        });
        let editable = (selected.is_some() || self.editing_rule.is_some())
            && self.calibration.is_none()
            && self.picker.is_none();
        let parameters_enabled = editable && self.mouse_shield;
        let validation = self.options(cx);
        let can_protect = selected.is_some()
            && editable
            && (self.system_menu || self.mouse_shield)
            && validation.is_ok();
        let count = self.table.read(cx).delegate().rows.len();
        let filtering = !self.search.read(cx).value().trim().is_empty();
        let count_label = if filtering {
            format!("{count}/{} 个窗口", self.windows.len())
        } else {
            format!("{} 个窗口", self.windows.len())
        };
        let validation_error: Option<SharedString> =
            if editable && !self.system_menu && !self.mouse_shield {
                Some("请选择至少一种保护方式。".into())
            } else if parameters_enabled {
                validation.err()
            } else {
                None
            };
        let action_hint = match selected {
            Some(target) if protected => {
                format!("解除“{}”的保护。", target.display_title())
            }
            Some(target) => format!("将当前设置应用于“{}”。", target.display_title()),
            None => "请先在表格中选择一个窗口。".into(),
        };
        let identity_label = self
            .editing_rule
            .and_then(|index| self.config.rules.get(index))
            .map(|rule| format!("程序：{}  ·  类名：{}", rule.path, rule.class))
            .or_else(|| {
                selected.map(|target| {
                    format!(
                        "程序：{}  ·  类名：{}",
                        self.path_for(target)
                            .unwrap_or("无法读取，请检查目标窗口权限"),
                        target.class
                    )
                })
            });
        let view = cx.entity();
        let control_height = px(30.);
        let number_width = px(96.);
        let fields = [
            (
                "width-field",
                "宽",
                &self.width,
                "遮罩宽度：1–512 DIP，按目标窗口 DPI 缩放。",
            ),
            ("height-field", "高", &self.height, "遮罩高度：1–512 DIP。"),
            (
                "right-field",
                "内缩",
                &self.right_offset,
                "右侧内缩：−512–512 DIP。正值向左，负值向右。",
            ),
            (
                "top-field",
                "下移",
                &self.top_offset,
                "顶部下移：−512–512 DIP。正值向下，负值向上。",
            ),
        ]
        .into_iter()
        .map(|(id, label, state, hint)| {
            div()
                .id(id)
                .flex()
                .flex_shrink_0()
                .items_center()
                .gap(px(6.))
                .h(control_height)
                .tooltip(move |window, cx| Tooltip::new(hint).build(window, cx))
                .child(div().text_sm().flex_shrink_0().child(label))
                .child(
                    div().w(number_width).flex_shrink_0().child(
                        NumberInput::new(state)
                            .small()
                            .h(control_height)
                            .disabled(!parameters_enabled),
                    ),
                )
        });

        let toolbar = div()
            .flex()
            .items_center()
            .gap_2()
            .flex_shrink_0()
            .child(div().flex_1().min_w_0().max_w(px(440.)).child(
                Input::new(&self.search)
                    .h(px(32.))
                    .disabled(self.picker.is_some())
                    .prefix(Icon::new(IconName::Search).small())
                    .cleanable(true),
            ))
            .child(div().flex_1())
            .child(Button::new("hint").ghost().icon(IconName::Lightbulb).accessibility_label("保护使用提示").tooltip("标准标题栏可使用系统关闭保护；自绘标题栏可开启鼠标遮罩。关闭或最小化主界面后，保护继续运行。通过托盘返回，退出时恢复全部窗口。"))
            .child(Button::new("refresh").ghost().icon(IconName::RefreshCw).label("刷新").tooltip("重新读取窗口列表").disabled(self.picker.is_some()).on_click(cx.listener(|this, _, _, cx| this.refresh(cx))))
            .child(Button::new("pick-window").ghost().icon(IconName::Crosshair).label("选择窗口").tooltip("移动准星到目标窗口，单击选择；Esc 取消").disabled(self.picker.is_some() || self.calibration.is_some()).on_click(cx.listener(|this, _, _, cx| this.start_picker(cx))))
            .child(Button::new("more").ghost().icon(IconName::Ellipsis).label("更多").disabled(self.picker.is_some()).dropdown_caret(false).dropdown_menu(move |menu, window, cx| {
                let restore_disabled = view.read(cx).protections.is_empty();
                menu.item(PopupMenuItem::new("恢复全部窗口").icon(IconName::RotateCcw).disabled(restore_disabled)
                    .on_click(window.listener_for(&view, |this, _, _, cx| this.restore_all(cx))))
                    .item(PopupMenuItem::new("应用规则管理").on_click(window.listener_for(&view, |this, _, _, cx| { this.rules_open = !this.rules_open; cx.notify(); })))
                    .item(PopupMenuItem::new("开机启动（后台运行）").checked(view.read(cx).startup_enabled).on_click(window.listener_for(&view, |this, _, _, cx| this.toggle_startup(cx))))
                    .separator()
                    .item(PopupMenuItem::new("退出并恢复").icon(IconName::LogOut)
                        .on_click(window.listener_for(&view, |this, _, _, cx| this.exit(cx))))
            }));

        let config = div()
            .v_flex()
            .gap_3()
            .p_3()
            .flex_shrink_0()
            .rounded(px(8.))
            .border_1()
            .border_color(theme.border)
            .bg(theme.table)
            .child(
                div().flex().items_center().gap_2()
                    .child(div().flex().items_center().gap_4().flex_shrink_0()
                        .child(Checkbox::new("system-menu").small().h(control_height).items_center().label("系统保护").checked(self.system_menu).disabled(!editable)
                            .tooltip("禁用标准标题栏和系统菜单的关闭命令。修改后点击“应用”。")
                            .on_change(cx.listener(|this, value, _, cx| { this.system_menu = *value; cx.notify(); })))
                        .child(Checkbox::new("mouse-shield").small().h(control_height).items_center().label("鼠标遮罩").checked(self.mouse_shield).disabled(!editable)
                            .tooltip("拦截遮罩区域内的鼠标点击，可用“校准”调整位置。修改后点击“应用”。")
                            .on_change(cx.listener(|this, value, _, cx| { this.mouse_shield = *value; cx.notify(); }))))
                    .when_some(identity_label, |row, label| row.child(Button::new("target-identity").ghost().small().h(control_height).icon(IconName::Info).accessibility_label("目标窗口信息").tooltip(label)))
                    .child(div().flex_1())
                    .child(Button::new("rule-settings").ghost().small().h(control_height).label("规则").icon(if self.rule_settings_open { IconName::ChevronUp } else { IconName::ChevronDown })
                        .tooltip(if self.rule_settings_open { "收起规则设置" } else { "展开标题匹配、自动保护和保存规则" }).disabled(!editable && !self.rule_settings_open)
                        .on_click(cx.listener(|this, _, _, cx| { this.rule_settings_open = !this.rule_settings_open; cx.notify(); })))
                    .when(protected, |row| row.child(Button::new("apply").small().h(control_height).label("应用").tooltip("将修改后的设置应用到当前窗口").disabled(!can_protect || recovery_failed).on_click(cx.listener(|this, _, _, cx| this.apply_settings(cx)))))
                    .child(Button::new("toggle-protection").primary().small().h(control_height).flex_shrink_0()
                        .icon(if protected { IconName::RotateCcw } else { IconName::ShieldCheck })
                        .label(if recovery_failed { "重试恢复" } else if protected { "解除保护" } else { "开启保护" })
                        .tooltip(action_hint).disabled(self.calibration.is_some() || self.picker.is_some() || (!protected && !can_protect))
                        .on_click(cx.listener(|this, _, _, cx| {
                            let protected = this.selected.as_ref().is_some_and(|target| this.protection_for(target).is_some());
                            if protected { this.restore_selected(cx); } else { this.protect(cx); }
                        })))
            )
            .when(self.mouse_shield, |config| config.child(
                div().flex().flex_wrap().items_center().gap_x_3().gap_y_2().children(fields)
                    .child(div().flex().items_center().gap_3().flex_shrink_0()
                        .child(div().id("opacity-field").flex().items_center().gap(px(6.)).h(control_height)
                            .tooltip(|window, cx| Tooltip::new("遮罩透明度：5–80%。数值越小，遮罩越透明。").build(window, cx))
                            .child(div().text_sm().child("透明度"))
                            .child(div().w(number_width).flex_shrink_0().child(NumberInput::new(&self.opacity).small().h(control_height).disabled(!parameters_enabled))))
                        .child(Button::new("calibrate").small().h(control_height).label("校准").tooltip("拖动遮罩调整位置和大小；Enter 保存，Esc 取消")
                            .disabled(!can_protect || recovery_failed).on_click(cx.listener(|this, _, _, cx| this.start_calibration(cx)))))
            ))
            .when(self.rule_settings_open, |config| config.child(div().flex().items_center().gap_3().pt_3().border_t_1().border_color(theme.border)
                .child(div().id("title-match-help").flex_1().min_w_0()
                    .tooltip(|window, cx| Tooltip::new("规则按程序路径和窗口类名匹配。标题关键词留空时匹配所有同类窗口。").build(window, cx))
                    .child(Input::new(&self.title_contains).small().h(control_height).aria_label("规则标题关键词").disabled(!editable)))
                .child(Checkbox::new("automatic").small().h(control_height).items_center().label("自动保护").checked(self.automatic).disabled(!editable)
                    .tooltip("保存后，自动保护匹配规则的新窗口")
                    .on_change(cx.listener(|this, checked, _, cx| { this.automatic = *checked; cx.notify(); })))
                .child(Button::new("save-rule").small().h(control_height).label("保存规则").tooltip("保存程序、窗口类名、标题条件和当前保护参数")
                    .disabled(!editable || validation_error.is_some() || self.editing_rule.is_none() && selected.is_none_or(|target| self.path_for(target).is_none()))
                    .on_click(cx.listener(|this, _, _, cx| this.save_rule(cx))))))
            .when_some(self.config_error.clone(), |config, error| config.child(div().text_xs().text_color(theme.danger).child(error)))
            .when_some(validation_error, |config, error| config.child(div().text_xs().text_color(theme.danger).child(error)));

        let rules = div()
            .id("rule-list")
            .flex_1()
            .min_h_0()
            .v_flex()
            .gap_2()
            .overflow_y_scroll()
            .children(self.config.rules.iter().enumerate().map(|(index, rule)| {
                let label = format!(
                    "{} · {} · {}",
                    rule.path.rsplit('\\').next().unwrap_or(&rule.path),
                    rule.class,
                    if rule.title_contains.is_empty() {
                        "所有同类窗口"
                    } else {
                        &rule.title_contains
                    }
                );
                div()
                    .flex()
                    .items_center()
                    .gap_2()
                    .child(div().flex_1().min_w_0().truncate().text_sm().child(label))
                    .child(
                        Button::new(("edit-rule", index))
                            .small()
                            .label("编辑")
                            .on_click(cx.listener(move |this, _, window, cx| {
                                this.edit_rule(index, window, cx)
                            })),
                    )
                    .child(
                        Button::new(("disable-rule", index))
                            .small()
                            .label(if rule.automatic {
                                "禁用自动"
                            } else {
                                "启用自动"
                            })
                            .on_click(cx.listener(move |this, _, _, cx| {
                                let mut config = this.config.clone();
                                config.rules[index].automatic = !config.rules[index].automatic;
                                this.commit_config(config);
                                cx.notify();
                            })),
                    )
                    .child(
                        Button::new(("delete-rule", index))
                            .small()
                            .label("删除")
                            .on_click(cx.listener(move |this, _, _, cx| {
                                let mut config = this.config.clone();
                                config.rules.remove(index);
                                this.commit_config(config);
                                this.editing_rule = None;
                                cx.notify();
                            })),
                    )
            }))
            .when(self.config.rules.is_empty(), |list| {
                list.child(div().text_sm().child("暂无规则，选中窗口后保存应用规则。"))
            });
        let table = div()
            .flex_1()
            .min_h(px(110.))
            .rounded(px(8.))
            .border_1()
            .border_color(theme.border)
            .bg(theme.table)
            .overflow_hidden()
            .child(
                DataTable::new(&self.table)
                    .bordered(false)
                    .with_size(px(32.))
                    .header_row_height(px(30.)),
            );

        div()
            .size_full()
            .v_flex()
            .bg(theme.background)
            .text_color(theme.foreground)
            .font_family(theme.font_family.clone())
            .child(
                div()
                    .flex_1()
                    .min_h_0()
                    .v_flex()
                    .gap_3()
                    .p_4()
                    .child(toolbar)
                    .child(if self.rules_open {
                        div()
                            .flex_1()
                            .min_h(px(110.))
                            .v_flex()
                            .gap_2()
                            .child(div().text_sm().child("应用规则"))
                            .child(rules)
                            .into_any_element()
                    } else {
                        table.into_any_element()
                    })
                    .child(config),
            )
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap_4()
                    .h(px(28.))
                    .px_4()
                    .flex_shrink_0()
                    .border_t_1()
                    .border_color(theme.border)
                    .bg(theme.table)
                    .text_xs()
                    .text_color(theme.muted_foreground)
                    .child(
                        div()
                            .flex_shrink_0()
                            .child(format!("{count_label} · {} 个保护", self.protections.len())),
                    )
                    .child(
                        div()
                            .id("operation-status")
                            .flex_1()
                            .min_w_0()
                            .text_right()
                            .truncate()
                            .child(self.status.clone())
                            .tooltip({
                                let status = self.status.clone();
                                move |window, cx| Tooltip::new(status.clone()).build(window, cx)
                            }),
                    ),
            )
    }
}

pub fn run() {
    let background = std::env::args().any(|arg| arg == "--background");
    let _instance = match crate::instance::Instance::acquire(background) {
        Ok(Some(instance)) => instance,
        Ok(None) => return,
        Err(error) => {
            report_startup_error(background, &error);
            return;
        }
    };
    let (recovery, recovery_message) = Recovery::begin();
    gpui_kit::application()
        .with_assets(gpui_kit::assets::AllAssets)
        .run(move |cx| {
            gpui_kit::init(cx);
            ui::init_theme(cx);
            if let Err(error) = crate::protection::register_overlay_class() {
                report_startup_error(background, &format!("无法注册鼠标遮罩：{error}"));
                cx.quit();
                return;
            }
            let options = WindowOptions {
                show: !background,
                focus: !background,
                window_bounds: Some(WindowBounds::Windowed(Bounds::centered(
                    None,
                    size(px(860.), px(620.)),
                    cx,
                ))),
                window_min_size: Some(size(px(740.), px(540.))),
                ..WindowOptions::default()
            };
            let opened = gpui_kit::open_window(options, cx, |window, cx| {
                cx.new(|cx| KeepWindow::new(window, cx, recovery.clone(), recovery_message.clone()))
            });
            let (handle, view) = match opened {
                Ok(opened) => opened,
                Err(error) => {
                    report_startup_error(
                        background,
                        &format!("无法创建 KeepWindow 主窗口：{error}"),
                    );
                    cx.quit();
                    return;
                }
            };
            if background && view.read(cx).tray.is_none() {
                report_startup_error(true, &view.read(cx).status);
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

fn report_startup_error(background: bool, error: &str) {
    if background {
        if let Some(root) = std::env::var_os("LOCALAPPDATA") {
            let path = std::path::PathBuf::from(root).join("KeepWindow/startup-error.log");
            let _ = crate::storage::atomic_write(&path, error.as_bytes());
        }
    } else {
        unsafe {
            let text = window::wide(error);
            windows::Win32::UI::WindowsAndMessaging::MessageBoxW(
                None,
                windows::core::PCWSTR(text.as_ptr()),
                windows::core::w!("KeepWindow"),
                windows::Win32::UI::WindowsAndMessaging::MB_OK
                    | windows::Win32::UI::WindowsAndMessaging::MB_ICONERROR,
            );
        }
    }
}
