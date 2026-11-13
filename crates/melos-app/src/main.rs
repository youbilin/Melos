#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod api;
mod app;
mod shortcuts;
mod smtc;
mod theme;
mod ui;

use gpui::*;

fn main() {
    smtc::set_app_user_model_id();
    gpui_platform::application().run(|cx: &mut App| {
        ui::input::bind_keys(cx);

        let bounds = Bounds::centered(None, size(px(1120.), px(720.)), cx);

        cx.open_window(
            WindowOptions {
                window_bounds: Some(WindowBounds::Windowed(bounds)),
                titlebar: Some(TitlebarOptions {
                    title: Some("Melos".into()),
                    appears_transparent: false,
                    traffic_light_position: None,
                }),
                ..Default::default()
            },
            |_, cx| cx.new(app::Root::new),
        )
        .expect("failed to open window");

        cx.activate(true);
    });
}
