//! Reusable native GPUI artwork surfaces.

use std::sync::Arc;

use gpui::{Div, RenderImage, div, img, prelude::*, px, rgb};

use crate::theme::Theme;

/// Builds a GPUI render-image from the artwork service's validated RGBA8 buffer.
/// GPUI stores render-image pixels in BGRA order, so the channel swap runs on
/// the background executor before the result reaches the UI thread.
pub(crate) fn render_image_from_artwork(
    width: u32,
    height: u32,
    rgba8: &[u8],
) -> Result<Arc<RenderImage>, &'static str> {
    let expected = (width as usize)
        .checked_mul(height as usize)
        .and_then(|pixels| pixels.checked_mul(4))
        .ok_or("image dimensions overflow")?;
    if width == 0 || height == 0 || rgba8.len() != expected {
        return Err("decoded artwork buffer does not match its dimensions");
    }

    let mut bgra = rgba8.to_vec();
    for pixel in bgra.chunks_exact_mut(4) {
        pixel.swap(0, 2);
    }
    let buffer = image::RgbaImage::from_raw(width, height, bgra)
        .ok_or("could not create a GPUI image frame")?;
    let frame = image::Frame::new(buffer);
    Ok(Arc::new(RenderImage::new(vec![frame])))
}

/// Reusable poster tile with deterministic loading and unavailable states.
pub(crate) fn poster_card(
    title: &str,
    detail: &str,
    image: Option<Arc<RenderImage>>,
    loading: bool,
) -> Div {
    let mut surface = div()
        .w(px(148.0))
        .h(px(222.0))
        .rounded_lg()
        .overflow_hidden()
        .bg(rgb(Theme::SURFACE_RAISED));

    if let Some(image) = image {
        surface = surface.child(img(image).w_full().h_full());
    } else {
        surface = surface.child(
            div()
                .size_full()
                .flex()
                .items_center()
                .justify_center()
                .bg(rgb(Theme::SURFACE_RAISED))
                .child(
                    div()
                        .text_xs()
                        .text_color(rgb(if loading {
                            Theme::ACCENT
                        } else {
                            Theme::TEXT_MUTED
                        }))
                        .child(if loading {
                            "LOADING ARTWORK"
                        } else {
                            "NO ARTWORK"
                        }),
                ),
        );
    }

    div()
        .w(px(148.0))
        .flex_shrink_0()
        .flex()
        .flex_col()
        .gap_2()
        .child(surface)
        .child(
            div()
                .text_sm()
                .text_color(rgb(Theme::TEXT_PRIMARY))
                .max_w(px(148.0))
                .child(title.to_owned()),
        )
        .child(
            div()
                .text_xs()
                .text_color(rgb(Theme::TEXT_MUTED))
                .child(detail.to_owned()),
        )
}

/// Wide native image surface for hero/backdrop layouts.
pub(crate) fn backdrop_surface(image: Option<Arc<RenderImage>>, height: f32, loading: bool) -> Div {
    let mut surface = div()
        .relative()
        .w_full()
        .h(px(height))
        .rounded_lg()
        .overflow_hidden()
        .bg(rgb(Theme::SURFACE));

    if let Some(image) = image {
        surface = surface.child(img(image).w_full().h_full());
    } else {
        surface = surface.child(
            div()
                .size_full()
                .flex()
                .items_center()
                .justify_center()
                .bg(rgb(Theme::SURFACE_RAISED))
                .child(
                    div()
                        .text_xs()
                        .text_color(rgb(Theme::TEXT_MUTED))
                        .child(if loading { "LOADING BACKDROP" } else { "SENZO" }),
                ),
        );
    }
    surface
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn converts_rgba_pixels_to_gpui_bgra_frames() {
        let image = render_image_from_artwork(1, 1, &[255, 32, 16, 255]).unwrap();
        assert_eq!(image.frame_count(), 1);
        assert_eq!(image.size(0).width.0, 1);
        assert_eq!(image.size(0).height.0, 1);
        assert_eq!(image.as_bytes(0).unwrap(), &[16, 32, 255, 255]);
    }

    #[test]
    fn rejects_a_buffer_that_does_not_match_its_dimensions() {
        assert!(render_image_from_artwork(2, 2, &[0; 4]).is_err());
        assert!(render_image_from_artwork(0, 1, &[]).is_err());
    }
}
