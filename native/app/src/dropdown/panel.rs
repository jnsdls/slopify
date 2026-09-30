//! What the Dropdown window shows: the player, the Picker in its place, or the Sign-in State,
//! over the footer. Ported from the Electron renderer's markup and `styles.css`.

use std::cell::Cell;
use std::rc::Rc;

use gpui::{
    AnyElement, AnyView, App, Bounds, Context, DispatchPhase, Div, Entity, FocusHandle, Focusable,
    FontFeatures, FontWeight, HighlightStyle, ImageSource, InteractiveText, MouseButton,
    MouseDownEvent, MouseMoveEvent, MouseUpEvent, ObjectFit, Pixels, Render, ScrollHandle,
    SharedString, Stateful, StyledText, Subscription, Transformation, WeakEntity, Window, canvas,
    div, img, linear_color_stop, linear_gradient, prelude::*, px, radians, relative, rgb, svg,
    white,
};
use slopify_spotify::Source;

use super::paste_field::{FieldColors, PasteField, PasteFieldEvent};
use super::theme::Theme;
use super::{Dismiss, Next, TogglePlay, WIDTH};
use crate::app_model::AppModel;
use crate::assets::Icon;
use crate::player::PlayerState;
use crate::player::format::{artist_url, format_time, track_url};
use crate::player::view::{
    SignInView, Status, controls, paste_error_line, picker_rows, progress_fraction, progress_line,
    same_source, source_name,
};

const MAX_HEIGHT: f32 = 560.0;
/// The range input's thumb is 12 px and stays inside the track, so its centre runs 6 px in.
const THUMB: f32 = 12.0;
/// The logo file's aspect ratio, for a box as wide as the CSS one with the height it gets.
const LOGO_ASPECT: f32 = 823.46 / 225.25;

pub struct Panel {
    focus: FocusHandle,
    model: Entity<AppModel>,
    paste: Entity<PasteField>,
    picker_open: bool,
    picker_scroll: ScrollHandle,
    paste_error: Option<&'static str>,
    /// The slider's value while the pointer holds it, so the model's echo cannot make it jump.
    dragging_volume: Option<f64>,
    slider_bounds: Rc<Cell<Option<Bounds<Pixels>>>>,
    artist_hovered: bool,
    art_url: Option<SharedString>,
    measured: Rc<Cell<Option<Pixels>>>,
    _subscriptions: Vec<Subscription>,
}

impl Panel {
    pub fn new(model: Entity<AppModel>, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let paste = cx.new(|cx| PasteField::new("Paste a playlist link", cx));
        let subscriptions = vec![
            cx.observe_window_activation(window, |this, window, cx| {
                if !window.is_window_active() {
                    this.close_picker(window, cx);
                    super::hide(cx);
                }
            }),
            cx.observe_window_appearance(window, |_, _, cx| cx.notify()),
            cx.observe(&model, |_, _, cx| cx.notify()),
            cx.subscribe_in(&paste, window, |this, _, event, window, cx| match event {
                PasteFieldEvent::Edited => {
                    if this.paste_error.take().is_some() {
                        cx.notify();
                    }
                }
                PasteFieldEvent::Submit(text) => this.submit_paste(text, window, cx),
            }),
        ];
        Self {
            focus: cx.focus_handle(),
            model,
            paste,
            picker_open: false,
            picker_scroll: ScrollHandle::new(),
            paste_error: None,
            dragging_volume: None,
            slider_bounds: Rc::default(),
            artist_hovered: false,
            art_url: None,
            measured: Rc::default(),
            _subscriptions: subscriptions,
        }
    }

    /// The Dropdown is about to show: start from the player, with the keys on the panel.
    pub fn on_show(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.close_picker(window, cx);
        window.focus(&self.focus, cx);
        // Sends the height again, in case the last one came before the window could take it.
        self.measured.set(None);
        cx.notify();
    }

    fn dismiss(&mut self, _: &Dismiss, window: &mut Window, cx: &mut Context<Self>) {
        if self.picker_open {
            self.close_picker(window, cx);
        } else {
            super::hide(cx);
        }
    }

    fn toggle_picker(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.picker_open {
            self.close_picker(window, cx);
            return;
        }
        self.picker_open = true;
        self.paste_error = None;
        self.paste.update(cx, |p, cx| p.clear(cx));
        self.model.update(cx, |m, cx| m.refresh_sources(cx));
        cx.notify();
    }

    fn close_picker(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if !self.picker_open {
            return;
        }
        self.picker_open = false;
        if self.paste.focus_handle(cx).is_focused(window) {
            window.focus(&self.focus, cx);
        }
        cx.notify();
    }

    fn choose(&mut self, source: Source, window: &mut Window, cx: &mut Context<Self>) {
        self.close_picker(window, cx);
        self.model.update(cx, |m, cx| m.start_source(source, cx));
    }

    fn submit_paste(&mut self, text: &str, window: &mut Window, cx: &mut Context<Self>) {
        let text = text.trim().to_string();
        if text.is_empty() {
            return;
        }
        self.paste.update(cx, |p, cx| p.set_disabled(true, cx));
        self.paste_error = None;
        let resolve = self
            .model
            .update(cx, |m, cx| m.resolve_pasted_link(text, cx));
        cx.spawn_in(window, async move |this, cx| {
            let result = resolve.await;
            let _ = this.update_in(cx, |this, window, cx| {
                this.paste.update(cx, |p, cx| p.set_disabled(false, cx));
                match result {
                    Ok(source) => this.choose(source, window, cx),
                    Err(code) => {
                        this.paste_error = Some(paste_error_line(code));
                        // The line lands under the field, below the fold of a long list.
                        this.picker_scroll.scroll_to_bottom();
                        if this.picker_open {
                            window.focus(&this.paste.focus_handle(cx), cx);
                        }
                    }
                }
                cx.notify();
            });
        })
        .detach();
    }

    fn start_volume_drag(&mut self, event: &MouseDownEvent, cx: &mut Context<Self>) {
        self.dragging_volume = Some(0.0);
        self.drag_volume(event.position.x, cx);
    }

    fn drag_volume(&mut self, x: Pixels, cx: &mut Context<Self>) {
        let (Some(_), Some(bounds)) = (self.dragging_volume, self.slider_bounds.get()) else {
            return;
        };
        let volume = volume_at(f32::from(x - bounds.left()), f32::from(bounds.size.width));
        if self.dragging_volume != Some(volume) {
            self.dragging_volume = Some(volume);
            self.model.update(cx, |m, cx| m.set_volume(volume, cx));
            cx.notify();
        }
    }

    fn end_volume_drag(&mut self, cx: &mut Context<Self>) {
        if self.dragging_volume.take().is_some() {
            cx.notify();
        }
    }

    /// Drops the previous artwork from GPUI's asset cache, which otherwise keeps every image.
    fn track_art(&mut self, url: Option<&str>, cx: &mut App) {
        if self.art_url.as_deref() == url {
            return;
        }
        if let Some(old) = self.art_url.take() {
            ImageSource::from(old).remove_asset(cx);
        }
        self.art_url = url.map(|u| SharedString::from(u.to_string()));
    }

    fn player(
        &mut self,
        s: &PlayerState,
        status: Status,
        sources: &[Source],
        theme: Theme,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Div {
        let middle = if self.picker_open {
            self.picker(s.source.as_ref(), sources, theme, window, cx)
                .into_any_element()
        } else {
            self.now(s, status, theme, cx).into_any_element()
        };
        div()
            .flex()
            .flex_col()
            .min_h_0()
            .child(middle)
            .child(self.source_row(s.source.as_ref(), theme, cx))
    }

    fn now(
        &mut self,
        s: &PlayerState,
        status: Status,
        theme: Theme,
        cx: &mut Context<Self>,
    ) -> Div {
        let track = s.track.as_ref();
        let art_url = track.and_then(|t| t.image_url.as_deref());
        self.track_art(art_url, cx);
        let enabled = controls(status);
        let playing = status == Status::Playing;

        let art = div()
            .size(px(272.))
            .rounded(px(4.))
            .overflow_hidden()
            .bg(theme.tile)
            .flex()
            .items_center()
            .justify_center()
            .child(match art_url {
                Some(url) => img(SharedString::from(url.to_string()))
                    .size_full()
                    .rounded(px(4.))
                    .object_fit(ObjectFit::Cover)
                    .into_any_element(),
                None => icon(Icon::AppIcon, 96.)
                    .text_color(theme.fg.opacity(0.3))
                    .into_any_element(),
            });

        let title = div()
            .id("title")
            .mt(px(12.))
            .text_size(px(15.))
            .font_weight(FontWeight::SEMIBOLD)
            .truncate();
        let title = match track {
            None => title.child("Pick something to play"),
            Some(t) => {
                let name = SharedString::from(t.name.clone());
                let title = title.tooltip(tooltip(name.clone(), theme)).child(name);
                match track_url(&t.uri) {
                    Some(url) => {
                        let model = self.model.clone();
                        title
                            .cursor_pointer()
                            .hover(|style| style.opacity(0.8))
                            .on_click(move |_, _, cx| model.read(cx).open_external(&url))
                    }
                    None => title,
                }
            }
        };

        let progress_area = div()
            .mt(px(8.))
            .min_h(px(21.))
            .child(
                div()
                    .h(px(3.))
                    .rounded(px(2.))
                    .bg(theme.track)
                    .overflow_hidden()
                    .child(
                        div()
                            .h_full()
                            .w(relative(progress_fraction(s.position_ms, s.duration_ms)))
                            .bg(theme.fg),
                    ),
            )
            .child(match progress_line(s) {
                Some((line, error)) => div()
                    .mt(px(4.))
                    .text_size(px(11.))
                    .text_center()
                    .truncate()
                    .text_color(if error { theme.error } else { theme.muted })
                    .child(line),
                None => {
                    let (position, duration) = match track {
                        Some(_) => (s.position_ms, s.duration_ms),
                        None => (0, 0),
                    };
                    div()
                        .mt(px(4.))
                        .flex()
                        .justify_between()
                        .text_size(px(10.))
                        .text_color(theme.muted)
                        .font_features(FontFeatures(std::sync::Arc::new(vec![("tnum".into(), 1)])))
                        .child(format_time(position))
                        .child(format_time(duration))
                }
            });

        let play = div()
            .id("play")
            .size(px(40.))
            .rounded_full()
            .bg(theme.play_bg)
            .flex()
            .items_center()
            .justify_center()
            .child(
                icon(if playing { Icon::Pause } else { Icon::Play }, 20.).text_color(theme.play_fg),
            );
        let play = if enabled.play {
            play.cursor_pointer().on_click(
                cx.listener(|this, _, _, cx| this.model.update(cx, |m, cx| m.toggle_play(cx))),
            )
        } else {
            play.opacity(0.35)
        };

        let next = div()
            .id("next")
            .size(px(32.))
            .rounded_full()
            .flex()
            .items_center()
            .justify_center()
            .child(icon(Icon::Next, 22.).text_color(theme.fg));
        let next = if enabled.next {
            next.cursor_pointer()
                .hover(move |style| style.bg(theme.hover))
                .on_click(cx.listener(|this, _, _, cx| this.model.update(cx, |m, cx| m.next(cx))))
        } else {
            next.opacity(0.35)
        };

        div()
            .flex()
            .flex_col()
            .pt(px(14.))
            .px(px(14.))
            .child(art)
            .child(title)
            .child(self.artists(s, theme, cx))
            .child(progress_area)
            .child(
                div()
                    .flex()
                    .items_center()
                    .justify_center()
                    .gap(px(22.))
                    .pt(px(8.))
                    .pb(px(10.))
                    // Keeps play centred without a previous button on the left.
                    .child(div().w(px(32.)))
                    .child(play)
                    .child(next),
            )
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(8.))
                    .pb(px(12.))
                    .child(icon(Icon::Volume, 14.).text_color(theme.muted))
                    .child(self.slider(s.volume, enabled.volume, theme, cx)),
            )
    }

    /// All artists joined with commas, the first one a link.
    fn artists(&self, s: &PlayerState, theme: Theme, cx: &mut Context<Self>) -> AnyElement {
        let Some(track) = s.track.as_ref().filter(|t| !t.artists.is_empty()) else {
            return div().into_any_element();
        };
        let names: Vec<&str> = track.artists.iter().map(|a| a.name.as_str()).collect();
        let text = SharedString::from(names.join(", "));
        let mut styled = StyledText::new(text.clone());
        let link = artist_url(&track.artists[0].uri).map(|url| (0..names[0].len(), url));
        if let Some((range, _)) = &link
            && self.artist_hovered
        {
            styled = styled.with_highlights([(
                range.clone(),
                HighlightStyle {
                    color: Some(theme.fg),
                    ..Default::default()
                },
            )]);
        }
        let mut line = InteractiveText::new("artists-text", styled);
        if let Some((range, url)) = link {
            let model = self.model.clone();
            let panel = cx.entity().downgrade();
            let first = range.clone();
            line = line
                .on_click(vec![range], move |_, _, cx| {
                    model.read(cx).open_external(&url)
                })
                .on_hover(move |index, _, _, cx| {
                    let hovered = index.is_some_and(|i| first.contains(&i));
                    let _ = panel.update(cx, |this, cx| {
                        if this.artist_hovered != hovered {
                            this.artist_hovered = hovered;
                            cx.notify();
                        }
                    });
                });
        }
        div()
            .id("artists")
            .mt(px(1.))
            .text_color(theme.muted)
            .truncate()
            .tooltip(tooltip(text, theme))
            .child(line)
            .into_any_element()
    }

    fn slider(
        &self,
        volume: f64,
        enabled: bool,
        theme: Theme,
        cx: &mut Context<Self>,
    ) -> Stateful<Div> {
        let v = self.dragging_volume.unwrap_or(volume).clamp(0.0, 1.0) as f32;
        let bounds = self.slider_bounds.clone();
        let dragging = self.dragging_volume.is_some();
        let panel = cx.entity().downgrade();
        let slider = div()
            .id("volume")
            .flex_1()
            .h(px(THUMB))
            .relative()
            .flex()
            .items_center()
            .child(
                div()
                    .w_full()
                    .h(px(4.))
                    .rounded(px(2.))
                    .bg(theme.track)
                    .child(div().h_full().w(relative(v)).rounded(px(2.)).bg(theme.fg)),
            )
            .child(
                div()
                    .absolute()
                    .top_0()
                    .bottom_0()
                    .left(px(THUMB / 2.))
                    .right(px(THUMB / 2.))
                    .child(
                        div()
                            .absolute()
                            .top_0()
                            .left(relative(v))
                            .ml(px(-THUMB / 2.))
                            .size(px(THUMB))
                            .rounded_full()
                            .bg(theme.fg),
                    ),
            )
            .child(
                canvas(
                    move |b, _, _| bounds.set(Some(b)),
                    move |_, _, window, _| {
                        if dragging {
                            follow_drag(panel, window);
                        }
                    },
                )
                .absolute()
                .size_full(),
            );
        if enabled {
            slider.on_mouse_down(
                MouseButton::Left,
                cx.listener(|this, event, _, cx| this.start_volume_drag(event, cx)),
            )
        } else {
            if dragging {
                // The volume went away under the pointer, e.g. another device took over.
                let panel = cx.entity().downgrade();
                cx.defer(move |cx| {
                    let _ = panel.update(cx, |this, cx| this.end_volume_drag(cx));
                });
            }
            slider.opacity(0.35)
        }
    }

    fn source_row(
        &self,
        source: Option<&Source>,
        theme: Theme,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let chevron = icon(Icon::Chevron, 18.).text_color(theme.muted);
        let chevron = if self.picker_open {
            chevron.with_transformation(Transformation::rotate(radians(std::f32::consts::PI)))
        } else {
            chevron
        };
        div()
            .id("source-row")
            .flex()
            .items_center()
            .gap(px(10.))
            .w_full()
            .px(px(14.))
            .py(px(10.))
            .border_t_1()
            .border_color(theme.line)
            .cursor_pointer()
            .hover(move |style| style.bg(theme.hover))
            .on_click(cx.listener(|this, _, window, cx| this.toggle_picker(window, cx)))
            .child(thumb(source, 32., theme))
            .child(
                div()
                    .flex()
                    .flex_col()
                    .flex_1()
                    .min_w_0()
                    .child(
                        div()
                            .text_size(px(10.))
                            .text_color(theme.muted)
                            .child("PLAYING FROM"),
                    )
                    .child(div().truncate().child(source_name(source).to_string())),
            )
            .child(chevron)
    }

    fn picker(
        &self,
        current: Option<&Source>,
        sources: &[Source],
        theme: Theme,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let rows = picker_rows(sources, current)
            .into_iter()
            .enumerate()
            .map(|(ix, source)| {
                let selected = same_source(Some(&source), current);
                let name = SharedString::from(source_name(Some(&source)).to_string());
                let thumb = thumb(Some(&source), 28., theme);
                div()
                    .id(("source", ix))
                    .flex()
                    .items_center()
                    .gap(px(10.))
                    .px(px(14.))
                    .py(px(6.))
                    .cursor_pointer()
                    .hover(move |style| style.bg(theme.hover))
                    .on_click(cx.listener(move |this, _, window, cx| {
                        this.choose(source.clone(), window, cx)
                    }))
                    .child(icon(Icon::Check, 14.).text_color(if selected {
                        theme.fg
                    } else {
                        gpui::transparent_black()
                    }))
                    .child(thumb)
                    .child(
                        div()
                            .id(("source-name", ix))
                            .min_w_0()
                            .truncate()
                            .tooltip(tooltip(name.clone(), theme))
                            .child(name),
                    )
            })
            .collect::<Vec<_>>();

        let paste = self.paste.read(cx);
        let disabled = paste.is_disabled();
        let focused = paste.focus_handle(cx).is_focused(window);
        self.paste.update(cx, |p, _| {
            p.set_colors(FieldColors {
                text: theme.fg,
                placeholder: theme.muted,
                selection: theme.selection,
            })
        });
        let field = div()
            .id("paste-box")
            .w_full()
            .bg(theme.field)
            .border_1()
            .border_color(if focused {
                theme.muted
            } else {
                theme.field_line
            })
            .rounded(px(6.))
            .px(px(8.))
            .py(px(5.))
            .when(disabled, |d| d.opacity(0.5))
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|this, _, window, cx| {
                    if !this.paste.read(cx).is_disabled() {
                        window.focus(&this.paste.focus_handle(cx), cx);
                    }
                }),
            )
            .child(self.paste.clone());

        div()
            .id("picker")
            .flex_auto()
            .min_h_0()
            .overflow_y_scroll()
            .track_scroll(&self.picker_scroll)
            .pb(px(4.))
            .child(div().py(px(6.)).children(rows))
            .child(
                div()
                    .px(px(14.))
                    .pt(px(6.))
                    .pb(px(8.))
                    .border_t_1()
                    .border_color(theme.line)
                    .child(field)
                    .when_some(self.paste_error, |d, line| {
                        d.child(
                            div()
                                .mt(px(4.))
                                .text_size(px(11.))
                                .text_color(theme.error)
                                .child(line),
                        )
                    }),
            )
    }

    fn sign_in(&self, view: SignInView, theme: Theme, cx: &mut Context<Self>) -> impl IntoElement {
        let button = div()
            .id("sign-in")
            .rounded_full()
            .px(px(18.))
            .py(px(8.))
            .bg(theme.play_bg)
            .text_color(theme.play_fg)
            .font_weight(FontWeight::SEMIBOLD)
            .child(if view.waiting {
                "Waiting for Spotify"
            } else {
                "Sign in with Spotify"
            });
        let button = if view.waiting {
            button.opacity(0.5)
        } else {
            button.cursor_pointer().on_click(
                cx.listener(|this, _, _, cx| this.model.update(cx, |m, cx| m.sign_in(cx))),
            )
        };
        div()
            .pt(px(28.))
            .px(px(20.))
            .pb(px(24.))
            .flex()
            .flex_col()
            .items_center()
            .text_center()
            .child(div().w_full().mb(px(16.)).child(view.line))
            .child(button)
    }

    fn footer(
        &self,
        display_name: Option<String>,
        theme: Theme,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        div()
            .flex()
            .justify_between()
            .items_center()
            .border_t_1()
            .border_color(theme.line)
            .text_size(px(11.))
            .text_color(theme.muted)
            // Clear space of half the icon height (12 px at this size) on every side of the logo.
            .pt(px(12.))
            .pb(px(12.))
            .pl(px(12.))
            .pr(px(14.))
            .child(
                div()
                    .flex_none()
                    .w(px(80.))
                    .h(px(24.))
                    .mr(px(12.))
                    .flex()
                    .items_center()
                    .child(
                        svg()
                            .path(Icon::SpotifyLogo.path())
                            .w(px(80.))
                            .h(px(80. / LOGO_ASPECT))
                            .text_color(theme.fg),
                    ),
            )
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(4.))
                    .min_w_0()
                    .when_some(display_name, |d, name| {
                        d.child(
                            div()
                                .max_w(px(120.))
                                .truncate()
                                .child(format!("{name} \u{b7}")),
                        )
                    })
                    .child(
                        div()
                            .id("quit")
                            .flex_none()
                            .cursor_pointer()
                            .hover(move |style| style.text_color(theme.fg))
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.model.update(cx, |m, cx| m.quit(cx))
                            }))
                            .child("Quit"),
                    ),
            )
    }
}

impl Render for Panel {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = Theme::for_appearance(window.appearance());
        let model = self.model.read(cx);
        let sign_in = model.sign_in_view();
        let state = model.player().clone();
        let status = model.status();
        let sources = model.sources().to_vec();
        let display_name = model.display_name().map(str::to_string);

        let body = match sign_in {
            Some(view) => {
                self.picker_open = false;
                self.sign_in(view, theme, cx).into_any_element()
            }
            None => self
                .player(&state, status, &sources, theme, window, cx)
                .into_any_element(),
        };

        let measured = self.measured.clone();
        let measure = canvas(
            move |bounds, _, cx| {
                let height = bounds.size.height;
                if measured.get() != Some(height) {
                    measured.set(Some(height));
                    super::set_content_height(f64::from(f32::from(height)), cx);
                }
            },
            |_, _, _, _| {},
        )
        .absolute()
        .top_0()
        .left_0()
        .size_full();

        div()
            .key_context("Dropdown")
            .track_focus(&self.focus)
            .on_action(cx.listener(Self::dismiss))
            .on_action(cx.listener(|this, _: &TogglePlay, _, cx| {
                this.model.update(cx, |m, cx| m.toggle_play(cx))
            }))
            .on_action(
                cx.listener(|this, _: &Next, _, cx| this.model.update(cx, |m, cx| m.next(cx))),
            )
            .size_full()
            .bg(theme.bg)
            .text_color(theme.fg)
            .font_family(".SystemUIFont")
            .text_size(px(13.))
            .line_height(relative(1.35))
            .child(
                div()
                    .relative()
                    .flex_none()
                    .w(px(WIDTH as f32))
                    .max_h(px(MAX_HEIGHT))
                    .flex()
                    .flex_col()
                    .child(body)
                    .child(self.footer(display_name, theme, cx))
                    .child(measure),
            )
    }
}

impl Focusable for Panel {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus.clone()
    }
}

/// While the slider is held, follows the pointer anywhere, in the window or out of it.
fn follow_drag(panel: WeakEntity<Panel>, window: &mut Window) {
    let moved = panel.clone();
    window.on_mouse_event(move |event: &MouseMoveEvent, phase, _, cx| {
        if phase == DispatchPhase::Bubble {
            let _ = moved.update(cx, |this, cx| this.drag_volume(event.position.x, cx));
        }
    });
    window.on_mouse_event(move |_: &MouseUpEvent, phase, _, cx| {
        if phase == DispatchPhase::Bubble {
            let _ = panel.update(cx, |this, cx| this.end_volume_drag(cx));
        }
    });
}

/// The slider value under `x`, measured from the track's left edge, in steps of 1 of 100.
fn volume_at(x: f32, width: f32) -> f64 {
    let travel = width - THUMB;
    if travel <= 0.0 {
        return 0.0;
    }
    let fraction = ((x - THUMB / 2.0) / travel).clamp(0.0, 1.0);
    (f64::from(fraction) * 100.0).round() / 100.0
}

fn icon(icon: Icon, size: f32) -> gpui::Svg {
    svg().path(icon.path()).size(px(size)).flex_none()
}

/// A Source's square tile: a playlist's cover, the heart for Liked Songs, empty for none.
fn thumb(source: Option<&Source>, size: f32, theme: Theme) -> Div {
    let tile = div()
        .size(px(size))
        .flex_none()
        .rounded(px(4.))
        .overflow_hidden()
        .flex()
        .items_center()
        .justify_center()
        .bg(theme.tile);
    match source {
        Some(Source::Liked) => tile
            .bg(linear_gradient(
                135.,
                linear_color_stop(rgb(0x4a3fd6), 0.),
                linear_color_stop(rgb(0xc6c1e8), 1.),
            ))
            .child(icon(Icon::Heart, 16.).text_color(white())),
        Some(Source::Playlist {
            image_url: Some(url),
            ..
        }) => tile.child(
            img(SharedString::from(url.clone()))
                .size_full()
                .rounded(px(4.))
                .object_fit(ObjectFit::Cover),
        ),
        _ => tile,
    }
}

struct Tooltip {
    text: SharedString,
    theme: Theme,
}

impl Render for Tooltip {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        div()
            .max_w(px(260.))
            .px(px(6.))
            .py(px(3.))
            .rounded(px(4.))
            .bg(self.theme.bg)
            .border_1()
            .border_color(self.theme.field_line)
            .text_color(self.theme.fg)
            .font_family(".SystemUIFont")
            .text_size(px(11.))
            .line_height(relative(1.35))
            .child(self.text.clone())
    }
}

/// The full string behind a truncated one, as the Electron app's `title` attribute gave it.
fn tooltip(text: SharedString, theme: Theme) -> impl Fn(&mut Window, &mut App) -> AnyView {
    move |_, cx| {
        cx.new(|_| Tooltip {
            text: text.clone(),
            theme,
        })
        .into()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn maps_the_pointer_to_the_thumb_centre() {
        // A 200 px track: the thumb centre runs from 6 to 194.
        assert_eq!(volume_at(6.0, 200.0), 0.0);
        assert_eq!(volume_at(100.0, 200.0), 0.5);
        assert_eq!(volume_at(194.0, 200.0), 1.0);
    }

    #[test]
    fn clamps_outside_the_track_and_rounds_to_whole_percent() {
        assert_eq!(volume_at(-40.0, 200.0), 0.0);
        assert_eq!(volume_at(400.0, 200.0), 1.0);
        assert_eq!(volume_at(7.0, 200.0), 0.01);
        assert_eq!(volume_at(5.0, 10.0), 0.0);
    }
}
