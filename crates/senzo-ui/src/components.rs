//! Reusable GPUI primitives for SENZO's shell and future feature screens.

use gpui::{Div, div, prelude::*, rgb};

use crate::theme::Theme;

#[derive(Clone, Copy)]
pub(crate) enum StatusTone {
    Info,
    Empty,
    Loading,
    Error,
}

pub(crate) fn page_intro(eyebrow: &str, title: &str, description: &str) -> Div {
    div()
        .flex()
        .flex_col()
        .gap_2()
        .child(
            div()
                .text_xs()
                .text_color(rgb(Theme::ACCENT))
                .child(eyebrow.to_uppercase()),
        )
        .child(
            div()
                .text_2xl()
                .text_color(rgb(Theme::TEXT_PRIMARY))
                .child(title.to_owned()),
        )
        .child(
            div()
                .text_sm()
                .text_color(rgb(Theme::TEXT_SECONDARY))
                .child(description.to_owned()),
        )
}

pub(crate) fn status_panel(tone: StatusTone, eyebrow: &str, title: &str, description: &str) -> Div {
    let (accent, marker) = match tone {
        StatusTone::Info => (Theme::INFO, "i"),
        StatusTone::Empty => (Theme::ACCENT, "—"),
        StatusTone::Loading => (Theme::WARNING, "…"),
        StatusTone::Error => (Theme::DANGER, "!"),
    };

    div()
        .w_full()
        .flex()
        .items_start()
        .gap_4()
        .rounded_lg()
        .border_1()
        .border_color(rgb(Theme::BORDER))
        .bg(rgb(Theme::SURFACE))
        .p_5()
        .child(
            div()
                .w(gpui::px(42.0))
                .h(gpui::px(42.0))
                .flex()
                .items_center()
                .justify_center()
                .rounded_md()
                .bg(rgb(Theme::ACCENT_DARK))
                .text_color(rgb(accent))
                .text_lg()
                .child(marker),
        )
        .child(
            div()
                .flex_1()
                .flex()
                .flex_col()
                .gap_2()
                .child(
                    div()
                        .text_xs()
                        .text_color(rgb(accent))
                        .child(eyebrow.to_uppercase()),
                )
                .child(
                    div()
                        .text_lg()
                        .text_color(rgb(Theme::TEXT_PRIMARY))
                        .child(title.to_owned()),
                )
                .child(
                    div()
                        .text_sm()
                        .text_color(rgb(Theme::TEXT_SECONDARY))
                        .child(description.to_owned()),
                ),
        )
}

pub(crate) fn feature_tile(eyebrow: &str, title: &str, description: &str) -> Div {
    div()
        .flex_1()
        .min_w(gpui::px(180.0))
        .flex()
        .flex_col()
        .gap_3()
        .rounded_lg()
        .border_1()
        .border_color(rgb(Theme::BORDER))
        .bg(rgb(Theme::SURFACE))
        .p_5()
        .child(
            div()
                .text_xs()
                .text_color(rgb(Theme::ACCENT))
                .child(eyebrow.to_uppercase()),
        )
        .child(
            div()
                .text_lg()
                .text_color(rgb(Theme::TEXT_PRIMARY))
                .child(title.to_owned()),
        )
        .child(
            div()
                .text_sm()
                .text_color(rgb(Theme::TEXT_SECONDARY))
                .child(description.to_owned()),
        )
}

pub(crate) fn divider() -> Div {
    div().w_full().h(gpui::px(1.0)).bg(rgb(Theme::BORDER))
}
