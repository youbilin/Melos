//! Spike C: gpui-pre + gpui-component (via the gpui-kit facade).
//!
//! Purpose: open a window with a real component-library input and check whether
//! Chinese IME works here. Compare binary size / startup / memory with spike A.

use gpui_kit::component::input::{Input, InputState};
use gpui_kit::*;

struct Demo {
    input: Entity<InputState>,
}

impl Render for Demo {
    fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .flex()
            .flex_col()
            .size_full()
            .gap_4()
            .p_8()
            .bg(rgb(0x0b0b0e))
            .text_color(rgb(0xf5f5f7))
            .child(
                div()
                    .text_2xl()
                    .child("Melos · Spike C（gpui-kit）"),
            )
            .child(div().text_sm().child("在这里输入中文，验证 IME："))
            .child(Input::new(&self.input).w_full())
    }
}

fn main() {
    gpui_kit::application().run(|cx: &mut App| {
        gpui_kit::init(cx);
        gpui_kit::open_window(WindowOptions::default(), cx, |window, cx| {
            let input = cx.new(|cx| InputState::new(window, cx).default_value("你好，世界"));
            cx.new(|_| Demo { input })
        })
        .expect("failed to open window");
        cx.activate(true);
    });
}
