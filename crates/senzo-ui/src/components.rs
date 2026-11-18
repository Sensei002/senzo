//! Reusable GPUI primitives for SENZO's shell and future feature screens.

use std::{collections::HashMap, sync::Arc};

use gpui::{Div, RenderImage, div, img, prelude::*, px, rgb};
use senzo_catalog::{
    HomeCollection, HomeCollectionFolder, HomeCollectionPosterShape, home_collection_folder_key,
    visible_home_collections,
};

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

/// Render a validated, horizontally-scrolling collection row using native GPUI cards.
/// Artwork is supplied as already-validated native RenderImage frames from senzo-artwork.
pub fn home_collection_row(
    collection: &HomeCollection,
    artwork: &HashMap<String, Arc<RenderImage>>,
) -> Div {
    let normalized = visible_home_collections(std::slice::from_ref(collection));
    let Some(collection) = normalized.first() else {
        return div();
    };

    let mut row = div().flex().gap_4();
    for folder in &collection.folders {
        let Some(key) = home_collection_folder_key(&collection.id, &folder.id) else {
            continue;
        };
        let image = artwork.get(&key).cloned();
        row = row.child(
            div()
                .id(key)
                .child(home_collection_folder_card(folder, image)),
        );
    }

    div()
        .w_full()
        .flex()
        .flex_col()
        .gap_3()
        .child(
            div()
                .text_lg()
                .text_color(rgb(Theme::TEXT_PRIMARY))
                .child(collection.title.clone()),
        )
        .child(div().w_full().overflow_x_scrollbar().child(row))
}

/// Reusable folder tile. The parent owns activation/navigation so the same
/// visual can be used by Home, Library and future collection destinations.
pub fn home_collection_folder_card(
    folder: &HomeCollectionFolder,
    image: Option<Arc<RenderImage>>,
) -> Div {
    let (width, height) = match folder.poster_shape {
        HomeCollectionPosterShape::Poster => (148.0, 222.0),
        HomeCollectionPosterShape::Landscape => (220.0, 124.0),
        HomeCollectionPosterShape::Square => (148.0, 148.0),
    };

    let mut surface = div()
        .w(px(width))
        .h(px(height))
        .rounded_lg()
        .overflow_hidden()
        .border_1()
        .border_color(rgb(Theme::BORDER))
        .bg(rgb(Theme::SURFACE_RAISED));

    if let Some(image) = image {
        surface = surface.child(img(image).w_full().h_full());
    } else {
        let fallback = folder
            .cover_emoji
            .as_deref()
            .filter(|value| !value.is_empty())
            .map(str::to_owned)
            .unwrap_or_else(|| {
                folder
                    .title
                    .chars()
                    .take(2)
                    .collect::<String>()
                    .to_uppercase()
            });
        surface = surface.child(
            div()
                .size_full()
                .flex()
                .items_center()
                .justify_center()
                .bg(rgb(Theme::SURFACE_RAISED))
                .child(
                    div()
                        .text_2xl()
                        .text_color(rgb(Theme::TEXT_SECONDARY))
                        .child(fallback),
                ),
        );
    }

    let mut card = div().w(px(width)).flex().flex_col().gap_2().child(surface);

    if !folder.hide_title {
        card = card.child(
            div()
                .max_w(px(width))
                .text_sm()
                .text_color(rgb(Theme::TEXT_PRIMARY))
                .child(folder.title.clone()),
        );
    }
    card
}

pub(crate) fn divider() -> Div {
    div().w_full().h(gpui::px(1.0)).bg(rgb(Theme::BORDER))
}
