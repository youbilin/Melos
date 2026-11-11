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
pub fn muted_label(text: impl Into<SharedString>) -> impl IntoElement {
    div()
        .text_xs()
        .text_color(rgb(theme::TEXT_FAINT))
        .child(text.into())
}
