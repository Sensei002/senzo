use gpui::{App, Context, QuitMode, Render, Window, WindowOptions, div, prelude::*, rgb};

struct SenzoApp;

impl Render for SenzoApp {
    fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .size_full()
            .flex()
            .items_center()
            .justify_center()
            .bg(rgb(0x0A0A0A))
            .child(
                div()
                    .text_color(rgb(0xF5F5F5))
                    .child("SENZO"),
            )
    }
}

fn main() {
    gpui_platform::application()
        .with_quit_mode(QuitMode::LastWindowClosed)
        .run(|cx: &mut App| {
            cx.open_window(WindowOptions::default(), |_window, cx| {
                cx.new(|_cx| SenzoApp)
            })
            .expect("failed to open SENZO window");

            cx.activate(true);
        });
}
