use gpui_kit::{
    App, Context, Div, InteractiveElement as _, IntoElement, ParentElement as _, Pixels, Stateful,
    StatefulInteractiveElement as _, Styled as _, Window,
    assets::IconName,
    base::StyledExt,
    component::{
        ActiveTheme, Icon, Sizable as _, Theme,
        table::{Column, TableDelegate, TableState},
        tag::Tag,
        tooltip::Tooltip,
    },
    div,
    prelude::FluentBuilder as _,
    px, rgb,
};

use crate::window::WindowInfo;

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

pub fn protection_tag(protected: bool, cx: &App) -> Tag {
    let theme = cx.theme();
    let tag = if protected {
        Tag::custom(theme.primary, theme.primary_foreground, theme.primary)
    } else {
        Tag::custom(theme.table, theme.muted_foreground, theme.border)
    };
    tag.small()
        .rounded_full()
        .gap_1()
        .when(protected, |tag| {
            tag.child(Icon::new(IconName::Lock).size(px(12.)))
        })
        .child(if protected { "已保护" } else { "未保护" })
}

pub struct WindowRow {
    pub info: WindowInfo,
    pub protection: Option<&'static str>,
}

pub struct WindowTable {
    pub rows: Vec<WindowRow>,
    pub filtered: bool,
    pub selected_row: Option<usize>,
    layout_width: Pixels,
    columns: Vec<Column>,
}

impl WindowTable {
    pub fn new() -> Self {
        Self {
            rows: Vec::new(),
            filtered: false,
            selected_row: None,
            layout_width: px(0.),
            columns: vec![
                Column::new("title", "窗口名称").width(430.).min_width(200.),
                Column::new("pid", "PID").width(86.).min_width(70.),
                Column::new("handle", "窗口句柄")
                    .width(138.)
                    .min_width(112.),
                Column::new("status", "保护状态")
                    .width(122.)
                    .resizable(false),
            ],
        }
    }

    pub fn fit_width(&mut self, width: Pixels) -> bool {
        if self.layout_width == width {
            return false;
        }
        self.layout_width = width;
        self.columns[0] = self.columns[0]
            .clone()
            .width((width - px(364.)).max(px(200.)));
        true
    }
}

impl TableDelegate for WindowTable {
    fn columns_count(&self, _: &App) -> usize {
        self.columns.len()
    }

    fn rows_count(&self, _: &App) -> usize {
        self.rows.len()
    }

    fn column(&self, col_ix: usize, _: &App) -> Column {
        self.columns[col_ix].clone()
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
        match col_ix {
            0 => {
                let title = row.info.display_title();
                cell.relative()
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
                    .child(
                        div()
                            .id(("window-title", row_ix))
                            .w_full()
                            .truncate()
                            .child(title.clone())
                            .tooltip(move |window, cx| {
                                Tooltip::new(title.clone()).build(window, cx)
                            }),
                    )
                    .into_any_element()
            }
            1 => cell
                .font_family("Consolas")
                .text_color(theme.muted_foreground)
                .child(row.info.pid.to_string())
                .into_any_element(),
            2 => cell
                .font_family("Consolas")
                .text_color(theme.muted_foreground)
                .child(format!("0x{:X}", row.info.hwnd.0 as usize))
                .into_any_element(),
            _ => {
                let description = row.protection.unwrap_or("尚未开启保护");
                let status = if row.protection.is_some() {
                    protection_tag(true, cx).into_any_element()
                } else {
                    div()
                        .text_xs()
                        .text_color(theme.muted_foreground)
                        .child("未保护")
                        .into_any_element()
                };
                cell.child(
                    div()
                        .id(("protection-tag", row_ix))
                        .child(status)
                        .tooltip(move |window, cx| Tooltip::new(description).build(window, cx)),
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
