//! Minimal widgets. The project deliberately owns these instead of pulling in
//! `gpui-component` (see ADR-001).

pub mod input;

use gpui::prelude::*;
use gpui::*;

use crate::theme;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Icon {
    Play,
    Pause,
    Next,
    Prev,
    Shuffle,
    Repeat,
    RepeatOne,
    Home,
    Artist,
    Album,
    Playlist,
    Songs,
    Search,
    Lyrics,
    Settings,
    SidebarExpand,
    SidebarCollapse,
}

pub fn icon_button(
    id: impl Into<ElementId>,
    icon: Icon,
    selected: bool,
    on_click: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
) -> impl IntoElement {
    let color = rgb(if selected {
        theme::TEXT
    } else {
        theme::TEXT_MUTED
    });
    div()
        .id(id)
        .size(px(36.))
        .flex_shrink_0()
        .flex()
        .items_center()
        .justify_center()
        .rounded_md()
        .cursor_pointer()
        .when(selected, |style| style.bg(rgba(0xFFFFFF14)))
        .hover(|style| style.bg(rgba(0xFFFFFF0A)))
        .child(
            canvas(
                move |_, _, _| {},
                move |bounds, _, window, _| {
                    paint_icon(icon, bounds, color, window);
                },
            )
            .size(px(20.)),
        )
        .on_click(on_click)
}

/// A flat text button used for nav entries and player controls.
pub fn ghost_button(
    id: impl Into<ElementId>,
    label: impl Into<SharedString>,
    selected: bool,
    on_click: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
) -> impl IntoElement {
    let base = div()
        .id(id)
        .flex()
        .items_center()
        .px_3()
        .py_2()
        .rounded_md()
        .text_sm()
        .cursor_pointer();

    let base = if selected {
        base.bg(rgba(0xFFFFFF14)).text_color(rgb(theme::TEXT))
    } else {
        base.text_color(rgb(theme::TEXT_MUTED))
    };

    base.hover(|style| style.bg(rgba(0xFFFFFF0A)))
        .child(label.into())
        .on_click(on_click)
}

/// A small text button that fits stacked controls in the playback bar.
pub fn compact_button(
    id: impl Into<ElementId>,
    label: impl Into<SharedString>,
    selected: bool,
    on_click: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
) -> impl IntoElement {
    div()
        .id(id)
        .px_2()
        .py_1()
        .rounded_md()
        .text_xs()
        .cursor_pointer()
        .text_color(rgb(if selected {
            theme::TEXT
        } else {
            theme::TEXT_MUTED
        }))
        .when(selected, |style| style.bg(rgba(0xFFFFFF14)))
        .hover(|style| style.bg(rgba(0xFFFFFF0A)))
        .child(label.into())
        .on_click(on_click)
}

/// A circular transport control with a vector icon (no emoji glyphs).
pub fn transport_button(
    id: &'static str,
    icon: Icon,
    primary: bool,
    on_click: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
) -> impl IntoElement {
    let diameter = if primary { 42. } else { 36. };
    let icon_size = if primary { 14. } else { 13. };
    let color = if primary {
        rgb(0x0B0B0C)
    } else {
        rgb(theme::TEXT_MUTED)
    };

    let base = div()
        .id(id)
        .flex()
        .items_center()
        .justify_center()
        .size(px(diameter))
        .rounded_full()
        .cursor_pointer();

    let base = if primary {
        base.bg(rgb(theme::TEXT))
    } else {
        base
    };

    base.hover(move |style| {
        if primary {
            style.opacity(0.88)
        } else {
            style.bg(rgb(theme::HOVER))
        }
    })
    .child(
        canvas(
            move |_, _, _| {},
            move |bounds, _, window, _| paint_icon(icon, bounds, color, window),
        )
        .size(px(icon_size)),
    )
    .on_click(on_click)
}

fn paint_icon(icon: Icon, bounds: Bounds<Pixels>, color: Rgba, window: &mut Window) {
    let at = |x: f32, y: f32| {
        point(
            bounds.origin.x + bounds.size.width * x,
            bounds.origin.y + bounds.size.height * y,
        )
    };

    match icon {
        Icon::Play => fill_polygon(
            &[at(0.28, 0.16), at(0.82, 0.5), at(0.28, 0.84)],
            color,
            window,
        ),
        Icon::Pause => {
            fill_rect(
                at(0.28, 0.16),
                bounds.size.width * 0.16,
                bounds.size.height * 0.68,
                color,
                window,
            );
            fill_rect(
                at(0.56, 0.16),
                bounds.size.width * 0.16,
                bounds.size.height * 0.68,
                color,
                window,
            );
        }
        Icon::Next => {
            fill_polygon(
                &[at(0.18, 0.16), at(0.66, 0.5), at(0.18, 0.84)],
                color,
                window,
            );
            fill_rect(
                at(0.70, 0.16),
                bounds.size.width * 0.14,
                bounds.size.height * 0.68,
                color,
                window,
            );
        }
        Icon::Prev => {
            fill_rect(
                at(0.16, 0.16),
                bounds.size.width * 0.14,
                bounds.size.height * 0.68,
                color,
                window,
            );
            fill_polygon(
                &[at(0.82, 0.16), at(0.34, 0.5), at(0.82, 0.84)],
                color,
                window,
            );
        }
        Icon::Shuffle => {
            stroke(
                &[
                    at(0.12, 0.24),
                    at(0.30, 0.24),
                    at(0.70, 0.76),
                    at(0.88, 0.76),
                ],
                color,
                window,
            );
            stroke(
                &[
                    at(0.12, 0.76),
                    at(0.30, 0.76),
                    at(0.70, 0.24),
                    at(0.88, 0.24),
                ],
                color,
                window,
            );
            stroke(
                &[at(0.72, 0.10), at(0.88, 0.24), at(0.72, 0.38)],
                color,
                window,
            );
            stroke(
                &[at(0.72, 0.62), at(0.88, 0.76), at(0.72, 0.90)],
                color,
                window,
            );
        }
        Icon::Repeat | Icon::RepeatOne => {
            stroke(
                &[
                    at(0.12, 0.46),
                    at(0.12, 0.28),
                    at(0.22, 0.18),
                    at(0.86, 0.18),
                ],
                color,
                window,
            );
            stroke(
                &[
                    at(0.88, 0.54),
                    at(0.88, 0.72),
                    at(0.78, 0.82),
                    at(0.14, 0.82),
                ],
                color,
                window,
            );
            stroke(
                &[at(0.72, 0.04), at(0.86, 0.18), at(0.72, 0.32)],
                color,
                window,
            );
            stroke(
                &[at(0.28, 0.68), at(0.14, 0.82), at(0.28, 0.96)],
                color,
                window,
            );
            if icon == Icon::RepeatOne {
                stroke(
                    &[at(0.42, 0.43), at(0.52, 0.37), at(0.52, 0.64)],
                    color,
                    window,
                );
            }
        }
        Icon::Home => {
            stroke(
                &[at(0.10, 0.44), at(0.50, 0.10), at(0.90, 0.44)],
                color,
                window,
            );
            stroke(
                &[
                    at(0.22, 0.34),
                    at(0.22, 0.88),
                    at(0.78, 0.88),
                    at(0.78, 0.34),
                ],
                color,
                window,
            );
        }
        Icon::Artist => {
            stroke(
                &[
                    at(0.38, 0.14),
                    at(0.62, 0.14),
                    at(0.68, 0.28),
                    at(0.62, 0.42),
                    at(0.38, 0.42),
                    at(0.32, 0.28),
                    at(0.38, 0.14),
                ],
                color,
                window,
            );
            stroke(
                &[
                    at(0.14, 0.90),
                    at(0.20, 0.64),
                    at(0.36, 0.54),
                    at(0.64, 0.54),
                    at(0.80, 0.64),
                    at(0.86, 0.90),
                ],
                color,
                window,
            );
        }
        Icon::Album => {
            stroke(
                &[
                    at(0.28, 0.10),
                    at(0.72, 0.10),
                    at(0.90, 0.28),
                    at(0.90, 0.72),
                    at(0.72, 0.90),
                    at(0.28, 0.90),
                    at(0.10, 0.72),
                    at(0.10, 0.28),
                    at(0.28, 0.10),
                ],
                color,
                window,
            );
            stroke(
                &[
                    at(0.42, 0.38),
                    at(0.58, 0.38),
                    at(0.62, 0.50),
                    at(0.58, 0.62),
                    at(0.42, 0.62),
                    at(0.38, 0.50),
                    at(0.42, 0.38),
                ],
                color,
                window,
            );
        }
        Icon::Playlist | Icon::Lyrics => {
            for y in [0.24, 0.50, 0.76] {
                stroke(&[at(0.14, y), at(0.86, y)], color, window);
            }
        }
        Icon::Songs => {
            stroke(
                &[at(0.40, 0.80), at(0.40, 0.16), at(0.84, 0.10)],
                color,
                window,
            );
            stroke(
                &[
                    at(0.30, 0.64),
                    at(0.46, 0.64),
                    at(0.50, 0.74),
                    at(0.46, 0.84),
                    at(0.30, 0.84),
                    at(0.26, 0.74),
                    at(0.30, 0.64),
                ],
                color,
                window,
            );
        }
        Icon::Search => {
            stroke(
                &[
                    at(0.22, 0.12),
                    at(0.54, 0.12),
                    at(0.68, 0.26),
                    at(0.68, 0.54),
                    at(0.54, 0.68),
                    at(0.22, 0.68),
                    at(0.08, 0.54),
                    at(0.08, 0.26),
                    at(0.22, 0.12),
                ],
                color,
                window,
            );
            stroke(&[at(0.62, 0.62), at(0.90, 0.90)], color, window);
        }
        Icon::Settings => {
            for (x, y) in [(0.24, 0.36), (0.50, 0.66), (0.76, 0.30)] {
                stroke(&[at(x, 0.10), at(x, y - 0.08)], color, window);
                stroke(&[at(x, y + 0.08), at(x, 0.90)], color, window);
                stroke(
                    &[
                        at(x - 0.09, y - 0.08),
                        at(x + 0.09, y - 0.08),
                        at(x + 0.09, y + 0.08),
                        at(x - 0.09, y + 0.08),
                        at(x - 0.09, y - 0.08),
                    ],
                    color,
                    window,
                );
            }
        }
        Icon::SidebarExpand | Icon::SidebarCollapse => {
            stroke(
                &[
                    at(0.10, 0.16),
                    at(0.90, 0.16),
                    at(0.90, 0.84),
                    at(0.10, 0.84),
                    at(0.10, 0.16),
                ],
                color,
                window,
            );
            stroke(&[at(0.34, 0.16), at(0.34, 0.84)], color, window);
            let (from, to) = if icon == Icon::SidebarExpand {
                (0.54, 0.72)
            } else {
                (0.74, 0.56)
            };
            stroke(
                &[at(from, 0.34), at(to, 0.50), at(from, 0.66)],
                color,
                window,
            );
        }
    }
}

fn stroke(points: &[Point<Pixels>], color: Rgba, window: &mut Window) {
    let Some((first, rest)) = points.split_first() else {
        return;
    };
    let mut builder = PathBuilder::stroke(px(1.6));
    builder.move_to(*first);
    for point in rest {
        builder.line_to(*point);
    }
    if let Ok(path) = builder.build() {
        window.paint_path(path, color);
    }
}

fn fill_polygon(points: &[Point<Pixels>], color: Rgba, window: &mut Window) {
    let Some((first, rest)) = points.split_first() else {
        return;
    };
    let mut builder = PathBuilder::fill();
    builder.move_to(*first);
    for point in rest {
        builder.line_to(*point);
    }
    builder.close();
    if let Ok(path) = builder.build() {
        window.paint_path(path, color);
    }
}

fn fill_rect(
    origin: Point<Pixels>,
    width: Pixels,
    height: Pixels,
    color: Rgba,
    window: &mut Window,
) {
    let body = quad(
        Bounds {
            origin,
            size: size(width, height),
        },
        px(1.),
        color,
        px(0.),
        gpui::transparent_black(),
        Default::default(),
    );
    window.paint_quad(body);
}

/// A filled primary action.
pub fn primary_button(
    label: impl Into<SharedString>,
    enabled: bool,
    on_click: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
) -> impl IntoElement {
    let label: SharedString = label.into();
    let id = label.to_string();

    let base = div()
        .id(id)
        .flex()
        .items_center()
        .justify_center()
        .w_full()
        .py_2()
        .rounded_md()
        .text_sm()
        .cursor_pointer();

    let base = if enabled {
        base.bg(rgb(theme::TEXT)).text_color(rgb(0x0B0B0C))
    } else {
        base.bg(rgb(theme::BORDER))
            .text_color(rgb(theme::TEXT_FAINT))
    };

    base.hover(move |style| if enabled { style.opacity(0.88) } else { style })
        .child(label)
        .on_click(move |event, window, cx| {
            if enabled {
                on_click(event, window, cx);
            }
        })
}

/// A small muted label used in the sidebar footer and settings.
pub fn empty_state(title: &'static str, hint: &'static str) -> impl IntoElement {
    div()
        .flex()
        .flex_col()
        .items_center()
        .justify_center()
        .gap_2()
        .h(px(200.))
        .rounded_md()
        .bg(rgba(0xFFFFFF08))
        .child(div().text_sm().text_color(rgb(0xF5F5F7)).child(title))
        .child(div().text_xs().text_color(rgb(0x9A9A9E)).child(hint))
}

pub fn muted_label(text: impl Into<SharedString>) -> impl IntoElement {
    div()
        .text_xs()
        .text_color(rgb(theme::TEXT_FAINT))
        .child(text.into())
}
