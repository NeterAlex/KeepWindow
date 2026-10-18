use std::{
    collections::{HashMap, HashSet},
    sync::Arc,
};

use gpui_kit::{
    App, Context, Div, InteractiveElement as _, IntoElement, ParentElement as _, Pixels,
    RenderImage, Stateful, StatefulInteractiveElement as _, Styled as _, Window,
    assets::IconName,
    base::StyledExt,
    component::{
        ActiveTheme, Icon, Sizable as _, Theme,
        table::{Column, TableDelegate, TableState},
        tag::Tag,
        tooltip::Tooltip,
    },
    div, img,
    prelude::FluentBuilder as _,
    px, rgb,
};

use crate::{
    icon::{WindowIconKey, load_window_icon},
    window::WindowInfo,
};

pub fn init_theme(cx: &mut App) {
    Theme::update(cx, |theme| {
        theme.font_family = "Microsoft YaHei UI".into();
        theme.mono_font_family = "Consolas".into();
        theme.radius = px(8.);
        theme.background = rgb(0xf7f7f5).into();
        theme.foreground = rgb(0x202020).into();
        theme.border = rgb(0xe3e3df).into();
        theme.muted_foreground = rgb(0x74746e).into();
        theme.secondary = rgb(0xefefec).into();
        theme.secondary_foreground = theme.foreground;
        theme.accent = rgb(0xececea).into();
        theme.accent_foreground = theme.foreground;
        theme.primary = rgb(0x242424).into();
        theme.primary_hover = rgb(0x3b3b3b).into();
        theme.primary_active = rgb(0x101010).into();
        theme.primary_foreground = rgb(0xffffff).into();
        theme.button_primary = theme.primary;
        theme.button_primary_hover = theme.primary_hover;
        theme.button_primary_active = theme.primary_active;
        theme.button_primary_foreground = theme.primary_foreground;
        theme.button = rgb(0xffffff).into();
        theme.button_foreground = theme.foreground;
        theme.button_hover = rgb(0xf1f1ee).into();
        theme.input = rgb(0xd9d9d4).into();
        theme.ring = rgb(0x777771).into();
        theme.table = rgb(0xffffff).into();
        theme.table_head = rgb(0xf5f5f2).into();
        theme.table_head_foreground = theme.muted_foreground;
        theme.table_row_border = rgb(0xefefeb).into();
        theme.table_hover = rgb(0xf7f7f4).into();
        theme.table_active = rgb(0xecece8).into();
        theme.table_active_border = rgb(0xecece8).into();
        theme.list.active_highlight = true;
    });
}

pub struct WindowRow {
    pub application: String,
    pub info: WindowInfo,
    pub protection: Option<(String, String)>,
}

pub struct WindowTable {
    pub advanced: bool,
    pub rows: Vec<WindowRow>,
    pub filtered: bool,
    pub selected_row: Option<usize>,
    layout_width: Pixels,
    columns: Vec<Column>,
    icons: HashMap<WindowIconKey, Option<Arc<RenderImage>>>,
}

impl WindowTable {
    pub fn new() -> Self {
        Self {
            advanced: false,
            rows: Vec::new(),
            filtered: false,
            selected_row: None,
            layout_width: px(0.),
            columns: vec![
                Column::new("application", "应用")
                    .width(150.)
                    .min_width(90.),
                Column::new("title", "窗口名称").width(430.).min_width(200.),
                Column::new("pid", "PID").width(86.).min_width(70.),
                Column::new("handle", "窗口句柄")
                    .width(138.)
                    .min_width(112.),
                Column::new("status", "保护状态")
                    .width(122.)
                    .resizable(false),
            ],
            icons: HashMap::new(),
        }
    }

    pub fn sync_icons(
        &mut self,
        targets: &[WindowInfo],
        window: &mut Window,
        cx: &mut Context<TableState<Self>>,
    ) {
        let current: HashSet<_> = targets.iter().map(WindowIconKey::from).collect();
        self.icons.retain(|key, image| {
            if current.contains(key) {
                return true;
            }
            if let Some(image) = image {
                cx.drop_image(image.clone(), Some(window));
            }
            false
        });
        for key in current {
            if self.icons.contains_key(&key) {
                continue;
            }
            self.icons.insert(key.clone(), None);
            cx.spawn_in(window, async move |table, cx| {
                let request = key.clone();
                let image = cx
                    .background_executor()
                    .spawn(async move { load_window_icon(&request) })
                    .await;
                let _ = table.update_in(cx, |table, _, cx| {
                    if let Some(cached) = table.delegate_mut().icons.get_mut(&key) {
                        *cached = image;
                        cx.notify();
                    }
                });
            })
            .detach();
        }
    }

    pub fn fit_width(&mut self, width: Pixels) -> bool {
        if self.layout_width == width {
            return false;
        }
        self.layout_width = width;
        self.columns[1] = self.columns[1]
            .clone()
            .width((width - px(if self.advanced { 514. } else { 272. })).max(px(200.)));
        true
    }
}

impl TableDelegate for WindowTable {
    fn columns_count(&self, _: &App) -> usize {
        if self.advanced { 5 } else { 3 }
    }

    fn rows_count(&self, _: &App) -> usize {
        self.rows.len()
    }

    fn column(&self, col_ix: usize, _: &App) -> Column {
        self.columns[if !self.advanced && col_ix == 2 {
            4
        } else {
            col_ix
        }]
        .clone()
    }

    fn render_tr(
        &mut self,
        row_ix: usize,
        _: &mut Window,
        _: &mut Context<TableState<Self>>,
    ) -> Stateful<Div> {
        div().id(("window-row", row_ix)).relative()
    }

    fn render_td(
        &mut self,
        row_ix: usize,
        col_ix: usize,
        _: &mut Window,
        cx: &mut Context<TableState<Self>>,
    ) -> impl IntoElement {
        let row = &self.rows[row_ix];
        let selected = self.selected_row == Some(row_ix);
        let theme = cx.theme();
        let cell = div().size_full().flex().items_center().text_sm();
        let column = if !self.advanced && col_ix == 2 {
            4
        } else {
            col_ix
        };
        match column {
            0 => cell
                .child(div().truncate().child(row.application.clone()))
                .into_any_element(),
            1 => {
                let title = row.info.display_title();
                let icon = div().size(px(16.)).flex_shrink_0().child(
                    self.icons
                        .get(&WindowIconKey::from(&row.info))
                        .and_then(Option::as_ref)
                        .map(|image| img(image.clone()).size_full().into_any_element())
                        .unwrap_or_else(|| {
                            Icon::new(IconName::AppWindow)
                                .size(px(16.))
                                .text_color(theme.muted_foreground)
                                .into_any_element()
                        }),
                );
                cell.relative()
                    .gap_2()
                    .min_w_0()
                    .when(selected, |cell| {
                        cell.child(
                            div()
                                .absolute()
                                .left(px(-12.))
                                .top_2()
                                .bottom_2()
                                .w(px(3.))
                                .rounded_full()
                                .bg(theme.foreground),
                        )
                    })
                    .child(icon)
                    .child(
                        div()
                            .id(("window-title", row_ix))
                            .flex_1()
                            .min_w_0()
                            .truncate()
                            .child(title.clone())
                            .tooltip(move |window, cx| {
                                Tooltip::new(title.clone()).build(window, cx)
                            }),
                    )
                    .into_any_element()
            }
            2 => cell
                .font_family("Consolas")
                .text_color(theme.muted_foreground)
                .child(row.info.pid.to_string())
                .into_any_element(),
            3 => cell
                .font_family("Consolas")
                .text_color(theme.muted_foreground)
                .child(format!("0x{:X}", row.info.hwnd.0 as usize))
                .into_any_element(),
            _ => {
                let description = row
                    .protection
                    .as_ref()
                    .map(|(_, detail)| detail.clone())
                    .unwrap_or_else(|| "尚未开启保护".into());
                let status = if let Some((label, _)) = &row.protection {
                    Tag::custom(theme.primary, theme.primary_foreground, theme.primary)
                        .small()
                        .rounded_full()
                        .child(label.clone())
                        .into_any_element()
                } else {
                    div()
                        .text_xs()
                        .text_color(theme.muted_foreground)
                        .child("未保护")
                        .into_any_element()
                };
                cell.child(
                    div().id(("protection-tag", row_ix)).child(status).tooltip(
                        move |window, cx| Tooltip::new(description.clone()).build(window, cx),
                    ),
                )
                .into_any_element()
            }
        }
    }

    fn render_empty(
        &mut self,
        _: &mut Window,
        cx: &mut Context<TableState<Self>>,
    ) -> impl IntoElement {
        div()
            .size_full()
            .v_flex()
            .items_center()
            .justify_center()
            .gap_2()
            .text_color(cx.theme().muted_foreground)
            .child(Icon::new(IconName::Search).size(px(24.)))
            .child(if self.filtered {
                "没有匹配的窗口"
            } else {
                "暂无可保护的窗口"
            })
            .child(div().text_sm().child(if self.filtered {
                "试试其他关键词，或清除搜索条件。"
            } else {
                "打开目标程序后，点击右上角刷新。"
            }))
    }
}
