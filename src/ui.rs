//! The picker panel: query field, emoji grid, recents on empty query.

use std::collections::HashMap;

use gpui::layer_shell::KeyboardInteractivity;
use gpui::prelude::*;
use gpui::{
    div, px, rgba, App, ClipboardItem, FocusHandle, KeyDownEvent, MouseButton, MouseDownEvent,
    SharedString, Window,
};

use crate::conf::PanelAnchor;
use crate::daemon::Daemon;
use crate::emoji_data::CATALOG;
use crate::search::index_by_cp;

/// Column count used for up/down grid navigation (matches the default
/// 640px width at 46px cells + gaps; approximate is fine).
const COLS: usize = 12;

pub struct PickerView {
    pub focus: FocusHandle,
    visible: bool,
    panel_w: f32,
    panel_h: f32,
    margin: f32,
    query: String,
    results: Vec<usize>,
    sel: usize,
    by_cp: HashMap<String, usize>,
    anchor: PanelAnchor,
    emoji_font: SharedString,
}

impl PickerView {
    pub fn new(cx: &mut Context<Self>) -> Self {
        let d = cx.global::<Daemon>();
        Self {
            focus: cx.focus_handle(),
            visible: false,
            panel_w: d.conf.panel.width as f32,
            panel_h: d.conf.panel.height as f32,
            margin: d.conf.panel.margin as f32,
            query: String::new(),
            results: Vec::new(),
            sel: 0,
            by_cp: index_by_cp(),
            anchor: d.conf.panel.anchor,
            emoji_font: d.emoji_font.clone(),
        }
    }

    /// Queue a search: the query line updates instantly, stale results stay
    /// visible, and the worker's answer lands via `apply_results`.
    fn refresh(&mut self, cx: &mut Context<Self>) {
        let d = cx.global_mut::<Daemon>();
        d.search_seq += 1;
        if self.query.is_empty() {
            self.results.clear();
            self.sel = 0;
            return;
        }
        let seq = d.search_seq;
        let _ = d.search_tx.send((seq, self.query.clone()));
    }

    /// Worker answer for the current query (seq-checked by the relay loop).
    pub fn apply_results(&mut self, results: Vec<usize>, cx: &mut Context<Self>) {
        self.results = results;
        self.sel = 0;
        cx.notify();
    }

    /// Show/hide the layer surface. Surface mutations (set_visible,
    /// set_keyboard_interactivity) silently no-op when issued from inside a
    /// window update, so hide always goes through `stow`'s deferred path.
    /// Mirrors upperadd's overlay lifecycle.
    pub fn apply_visibility(&mut self, visible: bool, window: &mut Window, cx: &mut Context<Self>) {
        self.visible = visible;
        window.set_visible(visible);
        window.set_keyboard_interactivity(if visible {
            KeyboardInteractivity::Exclusive
        } else {
            KeyboardInteractivity::None
        });
        if visible {
            window.focus(&self.focus, cx);
            self.query.clear();
            self.refresh(cx);
        }
        cx.notify();
    }

    /// Hide via a deferred surface mutation (safe from key/mouse handlers).
    fn stow(&mut self, cx: &mut Context<Self>) {
        if !self.visible {
            return;
        }
        self.visible = false;
        cx.defer(|app: &mut App| {
            app.global_mut::<Daemon>().open = false;
            if let Some(h) = app.global::<Daemon>().handle.clone() {
                let _ = h.update(app, |v, window, cx| v.apply_visibility(false, window, cx));
            }
        });
        cx.notify();
    }

    /// Entries on screen: recents when the query is empty, else results.
    fn visible(&self, cx: &Context<Self>) -> Vec<usize> {
        if self.query.is_empty() {
            cx.global::<Daemon>()
                .recents
                .items
                .iter()
                .filter_map(|cp| self.by_cp.get(cp).copied())
                .collect()
        } else {
            self.results.clone()
        }
    }

    fn on_key(&mut self, ev: &KeyDownEvent, _window: &mut Window, cx: &mut Context<Self>) {
        let k = &ev.keystroke;
        match k.key.as_str() {
            "escape" => self.stow(cx),
            "enter" => {
                let vis = self.visible(cx);
                if let Some(&idx) = vis.get(self.sel) {
                    self.copy_idx(idx, cx);
                }
            }
            "backspace" => {
                if self.query.pop().is_some() {
                    self.refresh(cx);
                }
            }
            "left" => self.sel = self.sel.saturating_sub(1),
            "right" => {
                let len = self.visible(cx).len();
                self.sel = (self.sel + 1).min(len.saturating_sub(1));
            }
            "up" => self.sel = self.sel.saturating_sub(COLS),
            "down" => {
                let len = self.visible(cx).len();
                self.sel = (self.sel + COLS).min(len.saturating_sub(1));
            }
            _ => {
                let m = &k.modifiers;
                if m.control || m.alt || m.platform || m.function {
                    return;
                }
                // key_char carries the typed character (IME/layout aware);
                // fall back to `key` for plain ASCII.
                let typed = k
                    .key_char
                    .clone()
                    .or_else(|| (k.key.chars().count() == 1).then(|| k.key.clone()));
                if let Some(c) = typed
                    .filter(|c| c.chars().count() == 1)
                    .filter(|c| !c.chars().next().unwrap().is_control())
                {
                    self.query.push_str(&c);
                    self.refresh(cx);
                }
            }
        }
        cx.notify();
    }

    fn copy_idx(&mut self, idx: usize, cx: &mut Context<Self>) {
        let e = &CATALOG[idx];
        cx.write_to_clipboard(ClipboardItem::new_string(e.ch.to_string()));
        {
            let d = cx.global_mut::<Daemon>();
            d.recents.push(e.cp);
            d.recents.save();
        }
        self.stow(cx);
    }

    fn render_cell(&self, pos: usize, idx: usize, cx: &mut Context<Self>) -> gpui::AnyElement {
        let selected = pos == self.sel;
        div()
            .id(pos)
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(move |this, _ev: &MouseDownEvent, _w, cx| this.copy_idx(idx, cx)),
            )
            .w(px(46.))
            .h(px(46.))
            .flex()
            .items_center()
            .justify_center()
            .rounded(px(8.))
            .cursor_pointer()
            .when(selected, |d| d.bg(rgba(0xffffff17)))
            .hover(|s| s.bg(rgba(0xffffff0d)))
            .text_size(px(28.))
            .font_family(self.emoji_font.clone())
            .child(CATALOG[idx].ch)
            .into_any_element()
    }
}

impl Render for PickerView {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let vis = self.visible(cx);
        let count = vis.len();
        let selected_name = vis
            .get(self.sel)
            .map(|&i| CATALOG[i].name)
            .unwrap_or_default();

        let cells: Vec<_> = vis
            .iter()
            .enumerate()
            .map(|(pos, &idx)| self.render_cell(pos, idx, cx))
            .collect();

        let query_text: SharedString = if self.query.is_empty() {
            "Search emoji…".into()
        } else {
            format!("{}▏", self.query).into()
        };

        // §16.3: the layer surface fills the whole output (all anchors, 0×0
        // bounds); the panel is positioned inside by flex + margins per the
        // 9-anchor config (default bottom-left).
        let a = self.anchor;
        let m = px(self.margin);
        div()
            .size_full()
            .flex()
            .flex_col()
            .when(a.v_center(), |d| d.justify_center())
            .when(a.is_bottom(), |d| d.justify_end())
            .when(a.h_center(), |d| d.items_center())
            .when(a.is_right(), |d| d.items_end())
            .when(a.is_top(), |d| d.mt(m))
            .when(a.is_bottom(), |d| d.mb(m))
            .when(a.is_left(), |d| d.ml(m))
            .when(a.is_right(), |d| d.mr(m))
            .child(
                div()
                    .id("panel")
                    .track_focus(&self.focus)
                    .key_context("Picker")
                    .on_key_down(cx.listener(Self::on_key))
                    .w(px(self.panel_w))
                    .h(px(self.panel_h))
                    .flex()
                    .flex_col()
                    .overflow_hidden()
                    .bg(rgba(0x17171bf2))
                    .border_1()
                    .border_color(rgba(0x2c2c34ff))
                    .rounded(px(14.))
                    .shadow_lg()
                    // header: query line
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .justify_between()
                            .px(px(14.))
                            .py(px(10.))
                            .child(
                                div()
                                    .text_size(px(16.))
                                    .text_color(if self.query.is_empty() {
                                        rgba(0x6b6b74ff)
                                    } else {
                                        rgba(0xe9e9eeff)
                                    })
                                    .child(query_text),
                            )
                            .child(
                                div()
                                    .text_size(px(11.))
                                    .text_color(rgba(0x6b6b74ff))
                                    .child("enter copy · esc close"),
                            ),
                    )
                    // grid
                    .child(
                        div()
                            .flex()
                            .flex_wrap()
                            .content_start()
                            .gap_1()
                            .px(px(10.))
                            .flex_1()
                            .overflow_hidden()
                            .when(count == 0, |d| {
                                d.items_center().justify_center().child(
                                    div().text_size(px(13.)).text_color(rgba(0x6b6b74ff)).child(
                                        if self.query.is_empty() {
                                            "type to search"
                                        } else {
                                            "no matches"
                                        },
                                    ),
                                )
                            })
                            .children(cells),
                    )
                    // footer: hovered/selected name
                    .child(
                        div()
                            .px(px(14.))
                            .py(px(7.))
                            .text_size(px(11.))
                            .text_color(rgba(0x8b8b93ff))
                            .child(selected_name),
                    ),
            )
    }
}
