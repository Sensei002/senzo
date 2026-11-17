//! Minimal native GPUI shell and GPUI-safe async task bridge.

use std::time::Duration;

use gpui::{
    App, Context, MouseButton, QuitMode, Render, Window, WindowOptions, div, prelude::*, rgb,
};
use senzo_domain::AppPage;
use tokio::runtime::Handle;

struct SenzoShell {
    active_page: AppPage,
    status: String,
    tokio: Handle,
    smoke_test: bool,
    smoke_test_scheduled: bool,
}

impl SenzoShell {
    fn new(tokio: Handle, smoke_test: bool) -> Self {
        Self {
            active_page: AppPage::Home,
            status: "Local-first foundation initialized".to_owned(),
            tokio,
            smoke_test,
            smoke_test_scheduled: false,
        }
    }

    fn nav_item(
        &self,
        label: &'static str,
        page: AppPage,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let active = self.active_page == page;
        div()
            .px_4()
            .py_2()
            .rounded_md()
            .cursor_pointer()
            .bg(if active { rgb(0x12313A) } else { rgb(0x171717) })
            .text_color(if active { rgb(0x22D3EE) } else { rgb(0xD4D4D8) })
            .child(label)
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(move |this, _event, _window, cx| {
                    this.active_page = page;
                    this.status = format!("Opened {} route", page.title());
                    cx.notify();
                }),
            )
    }

    fn check_runtime(&mut self, cx: &mut Context<Self>) {
        let runtime = self.tokio.clone();
        self.status = "Running a background Tokio task…".to_owned();
        cx.notify();

        cx.spawn(async move |this, cx| {
            let result = runtime
                .spawn(async {
                    tokio::time::sleep(Duration::from_millis(80)).await;
                    "Tokio worker completed successfully"
                })
                .await;
            let status = match result {
                Ok(message) => message.to_owned(),
                Err(error) => format!("Tokio worker failed: {error}"),
            };
            let _ = this.update(cx, |this, cx| {
                this.status = status;
                cx.notify();
            });
        })
        .detach();
    }
}

impl Render for SenzoShell {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        if self.smoke_test && !self.smoke_test_scheduled {
            self.smoke_test_scheduled = true;
            cx.spawn(async move |this, cx| {
                cx.background_executor()
                    .timer(Duration::from_millis(900))
                    .await;
                let _ = this.update(cx, |_, cx| {
                    tracing::info!("native GPUI window opened and rendered; ending smoke test");
                    cx.quit();
                });
            })
            .detach();
        }

        let page_title = self.active_page.title();
        let page_description = self.active_page.description();
        let status = self.status.clone();

        div()
            .size_full()
            .flex()
            .flex_col()
            .gap_5()
            .bg(rgb(0x0A0A0A))
            .text_color(rgb(0xF5F5F5))
            .p_6()
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap_1()
                    .child(div().text_2xl().text_color(rgb(0x22D3EE)).child("SENZO"))
                    .child(
                        div()
                            .text_sm()
                            .text_color(rgb(0xA1A1AA))
                            .child("Native Rust media center · Phase 1 foundation"),
                    ),
            )
            .child(
                div()
                    .flex()
                    .gap_2()
                    .child(self.nav_item("Home", AppPage::Home, cx))
                    .child(self.nav_item("Library", AppPage::Library, cx))
                    .child(self.nav_item("Settings", AppPage::Settings, cx)),
            )
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap_3()
                    .child(div().text_xl().child(page_title))
                    .child(div().text_color(rgb(0xA1A1AA)).child(page_description))
                    .child(
                        div()
                            .mt_3()
                            .px_4()
                            .py_3()
                            .rounded_md()
                            .bg(rgb(0x171717))
                            .child(status),
                    )
                    .child(
                        div()
                            .mt_2()
                            .px_4()
                            .py_2()
                            .rounded_md()
                            .bg(rgb(0x12313A))
                            .text_color(rgb(0x22D3EE))
                            .cursor_pointer()
                            .child("Check async runtime")
                            .on_mouse_down(
                                MouseButton::Left,
                                cx.listener(|this, _event, _window, cx| {
                                    this.check_runtime(cx);
                                }),
                            ),
                    ),
            )
    }
}

pub fn run(tokio: Handle, smoke_test: bool) {
    gpui_platform::application()
        .with_quit_mode(QuitMode::LastWindowClosed)
        .run(move |cx: &mut App| {
            cx.open_window(WindowOptions::default(), move |_window, cx| {
                cx.new(|_cx| SenzoShell::new(tokio, smoke_test))
            })
            .expect("failed to open the SENZO native window");
            cx.activate(true);
        });
}
