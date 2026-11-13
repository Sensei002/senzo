use gpui::{
    App, ClipboardItem, Context, QuitMode, Render, Window, WindowOptions, div, prelude::*, px, rgb,
};

use senzo::{
    diagnostics,
    settings::{
        DecoderPreference, HardwareDecoding, HdrMode, PlaybackSettings, RendererPreference,
        ScalingAlgorithm,
    },
    storage::LocalDatabase,
};

struct SenzoApp {
    database: LocalDatabase,
    settings: PlaybackSettings,
    status: String,
}

impl SenzoApp {
    fn new(database: LocalDatabase) -> Self {
        let settings = match database.playback_settings() {
            Ok(settings) => settings,
            Err(error) => {
                diagnostics::record(
                    diagnostics::Level::Error,
                    "settings",
                    &format!("failed to load playback preferences: {error:#}"),
                );
                PlaybackSettings::default()
            }
        };

        Self {
            database,
            settings,
            status: "Changes are saved on this device".into(),
        }
    }

    fn persist(&mut self, cx: &mut Context<Self>) {
        match self.database.save_playback_settings(&self.settings) {
            Ok(()) => self.status = "Saved locally".into(),
            Err(error) => {
                self.status = format!("Could not save settings: {error}");
                diagnostics::record(
                    diagnostics::Level::Error,
                    "settings",
                    &format!("failed to save playback preferences: {error:#}"),
                );
            }
        }
        cx.notify();
    }

    fn copy_diagnostics(&mut self, cx: &mut Context<Self>) {
        match diagnostics::collect_recent_logs() {
            Ok(report) => {
                cx.write_to_clipboard(ClipboardItem::new_string(report));
                self.status = "Diagnostic report copied to clipboard".into();
                diagnostics::record(
                    diagnostics::Level::Audit,
                    "diagnostics",
                    "user copied a diagnostic report",
                );
            }
            Err(error) => {
                self.status = format!("Could not collect logs: {error}");
                diagnostics::record(
                    diagnostics::Level::Error,
                    "diagnostics",
                    &format!("failed to collect diagnostic report: {error:#}"),
                );
            }
        }
        cx.notify();
    }
}

fn choice_chip<F>(
    label: &'static str,
    selected: bool,
    action: F,
    cx: &mut Context<SenzoApp>,
) -> impl IntoElement
where
    F: Fn(&mut SenzoApp, &mut Context<SenzoApp>) + 'static,
{
    let background = if selected {
        rgb(0x12343A)
    } else {
        rgb(0x202327)
    };
    let foreground = if selected {
        rgb(0x67E8F9)
    } else {
        rgb(0xD4D4D8)
    };

    div()
        .px_3()
        .py_2()
        .rounded_md()
        .bg(background)
        .text_color(foreground)
        .text_sm()
        .cursor_pointer()
        .hover(|style| style.bg(rgb(0x30363B)))
        .child(label)
        .on_mouse_down(
            gpui::MouseButton::Left,
            cx.listener(move |this, _event, _window, cx| {
                action(this, cx);
                this.persist(cx);
            }),
        )
}

fn section(
    title: &'static str,
    description: &'static str,
    content: impl IntoElement,
) -> impl IntoElement {
    div()
        .flex()
        .flex_col()
        .gap_3()
        .p_5()
        .rounded_lg()
        .bg(rgb(0x151719))
        .child(
            div()
                .flex()
                .flex_col()
                .gap_1()
                .child(
                    div()
                        .text_base()
                        .text_color(rgb(0xF4F4F5))
                        .font_weight(gpui::FontWeight::SEMIBOLD)
                        .child(title),
                )
                .child(div().text_sm().text_color(rgb(0x9299A1)).child(description)),
        )
        .child(content)
}

fn option_row(
    label: &'static str,
    description: &'static str,
    choices: impl IntoElement,
) -> impl IntoElement {
    div()
        .flex()
        .flex_col()
        .gap_2()
        .child(
            div()
                .flex()
                .flex_col()
                .gap_1()
                .child(div().text_sm().text_color(rgb(0xE4E4E7)).child(label))
                .child(div().text_xs().text_color(rgb(0x858C94)).child(description)),
        )
        .child(div().flex().flex_wrap().gap_2().child(choices))
}

impl Render for SenzoApp {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let settings = self.settings.clone();

        div()
            .id("senzo-root")
            .size_full()
            .flex()
            .bg(rgb(0x0A0C0E))
            .text_color(rgb(0xF4F4F5))
            .child(
                div()
                    .w(px(218.))
                    .h_full()
                    .flex()
                    .flex_col()
                    .gap_6()
                    .p_5()
                    .bg(rgb(0x101315))
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap_2()
                            .child(
                                div()
                                    .size(px(9.))
                                    .rounded_full()
                                    .bg(rgb(0x22D3EE)),
                            )
                            .child(
                                div()
                                    .text_xl()
                                    .font_weight(gpui::FontWeight::BOLD)
                                    .text_color(rgb(0xF8FAFC))
                                    .child("SENZO"),
                            ),
                    )
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .gap_2()
                            .child(nav_item("Home", false))
                            .child(nav_item("Discover", false))
                            .child(nav_item("Library", false))
                            .child(nav_item("Add-ons", false))
                            .child(nav_item("Settings", true)),
                    )
                    .child(div().flex_1())
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .gap_1()
                            .child(
                                div()
                                    .text_xs()
                                    .text_color(rgb(0x858C94))
                                    .child("LOCAL-FIRST"),
                            )
                            .child(
                                div()
                                    .text_xs()
                                    .text_color(rgb(0xB7BEC6))
                                    .child("Your settings stay on this device."),
                            ),
                    ),
            )
            .child(
                div()
                    .id("settings-content")
                    .flex_1()
                    .h_full()
                    .overflow_y_scroll()
                    .p_8()
                    .flex()
                    .flex_col()
                    .gap_5()
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .gap_2()
                            .child(
                                div()
                                    .text_2xl()
                                    .font_weight(gpui::FontWeight::BOLD)
                                    .child("Playback settings"),
                            )
                            .child(
                                div()
                                    .text_sm()
                                    .text_color(rgb(0x9299A1))
                                    .child("Tune decoding, image processing and playback resources."),
                            ),
                    )
                    .child(section(
                        "Hardware acceleration",
                        "Automatic is recommended. Manual decoder choices are advanced compatibility overrides.",
                        div()
                            .flex()
                            .flex_col()
                            .gap_4()
                            .child(option_row(
                                "Hardware decoding",
                                "Choose whether the playback engine may use your GPU decoder.",
                                div()
                                    .flex()
                                    .flex_wrap()
                                    .gap_2()
                                    .child(choice_chip(
                                        "Auto",
                                        settings.hardware_decoding == HardwareDecoding::Auto,
                                        |this, _cx| this.settings.hardware_decoding = HardwareDecoding::Auto,
                                        cx,
                                    ))
                                    .child(choice_chip(
                                        "Enabled",
                                        settings.hardware_decoding == HardwareDecoding::Enabled,
                                        |this, _cx| this.settings.hardware_decoding = HardwareDecoding::Enabled,
                                        cx,
                                    ))
                                    .child(choice_chip(
                                        "Disabled",
                                        settings.hardware_decoding == HardwareDecoding::Disabled,
                                        |this, _cx| this.settings.hardware_decoding = HardwareDecoding::Disabled,
                                        cx,
                                    )),
                            ))
                            .child(option_row(
                                "Decoder preference",
                                "D3D11VA is the preferred Windows path. DXVA2 remains for compatibility.",
                                div()
                                    .flex()
                                    .flex_wrap()
                                    .gap_2()
                                    .child(choice_chip(
                                        "Automatic",
                                        settings.decoder == DecoderPreference::Auto,
                                        |this, _cx| this.settings.decoder = DecoderPreference::Auto,
                                        cx,
                                    ))
                                    .child(choice_chip(
                                        "D3D11VA",
                                        settings.decoder == DecoderPreference::D3d11va,
                                        |this, _cx| this.settings.decoder = DecoderPreference::D3d11va,
                                        cx,
                                    ))
                                    .child(choice_chip(
                                        "DXVA2",
                                        settings.decoder == DecoderPreference::Dxva2,
                                        |this, _cx| this.settings.decoder = DecoderPreference::Dxva2,
                                        cx,
                                    ))
                                    .child(choice_chip(
                                        "Software",
                                        settings.decoder == DecoderPreference::Software,
                                        |this, _cx| this.settings.decoder = DecoderPreference::Software,
                                        cx,
                                    )),
                            ))
                            .child(option_row(
                                "Video renderer",
                                "Renderer selection will be applied when the playback backend is connected.",
                                div()
                                    .flex()
                                    .flex_wrap()
                                    .gap_2()
                                    .child(choice_chip(
                                        "Automatic",
                                        settings.renderer == RendererPreference::Auto,
                                        |this, _cx| this.settings.renderer = RendererPreference::Auto,
                                        cx,
                                    ))
                                    .child(choice_chip(
                                        "GPU Next",
                                        settings.renderer == RendererPreference::GpuNext,
                                        |this, _cx| this.settings.renderer = RendererPreference::GpuNext,
                                        cx,
                                    ))
                                    .child(choice_chip(
                                        "GPU",
                                        settings.renderer == RendererPreference::Gpu,
                                        |this, _cx| this.settings.renderer = RendererPreference::Gpu,
                                        cx,
                                    )),
                            )),
                    ))
                    .child(section(
                        "Image quality",
                        "Balanced defaults keep expensive processing disabled until you choose it.",
                        div()
                            .flex()
                            .flex_col()
                            .gap_4()
                            .child(option_row(
                                "Upscaling algorithm",
                                "Used when the source image is smaller than the display.",
                                div()
                                    .flex()
                                    .flex_wrap()
                                    .gap_2()
                                    .child(choice_chip("Bilinear", settings.upscale == ScalingAlgorithm::Bilinear, |this, _cx| this.settings.upscale = ScalingAlgorithm::Bilinear, cx))
                                    .child(choice_chip("Bicubic", settings.upscale == ScalingAlgorithm::Bicubic, |this, _cx| this.settings.upscale = ScalingAlgorithm::Bicubic, cx))
                                    .child(choice_chip("Lanczos", settings.upscale == ScalingAlgorithm::Lanczos, |this, _cx| this.settings.upscale = ScalingAlgorithm::Lanczos, cx))
                                    .child(choice_chip("Spline36", settings.upscale == ScalingAlgorithm::Spline36, |this, _cx| this.settings.upscale = ScalingAlgorithm::Spline36, cx))
                                    .child(choice_chip("Mitchell", settings.upscale == ScalingAlgorithm::Mitchell, |this, _cx| this.settings.upscale = ScalingAlgorithm::Mitchell, cx)),
                            ))
                            .child(option_row(
                                "Downscaling algorithm",
                                "Used when the source image is larger than the display.",
                                div()
                                    .flex()
                                    .flex_wrap()
                                    .gap_2()
                                    .child(choice_chip("Bilinear", settings.downscale == ScalingAlgorithm::Bilinear, |this, _cx| this.settings.downscale = ScalingAlgorithm::Bilinear, cx))
                                    .child(choice_chip("Bicubic", settings.downscale == ScalingAlgorithm::Bicubic, |this, _cx| this.settings.downscale = ScalingAlgorithm::Bicubic, cx))
                                    .child(choice_chip("Lanczos", settings.downscale == ScalingAlgorithm::Lanczos, |this, _cx| this.settings.downscale = ScalingAlgorithm::Lanczos, cx))
                                    .child(choice_chip("Spline36", settings.downscale == ScalingAlgorithm::Spline36, |this, _cx| this.settings.downscale = ScalingAlgorithm::Spline36, cx))
                                    .child(choice_chip("Mitchell", settings.downscale == ScalingAlgorithm::Mitchell, |this, _cx| this.settings.downscale = ScalingAlgorithm::Mitchell, cx)),
                            ))
                            .child(option_row(
                                "HDR handling",
                                "The playback engine will validate the display and renderer before applying HDR output.",
                                div()
                                    .flex()
                                    .flex_wrap()
                                    .gap_2()
                                    .child(choice_chip("Auto", settings.hdr == HdrMode::Auto, |this, _cx| this.settings.hdr = HdrMode::Auto, cx))
                                    .child(choice_chip("Tone map", settings.hdr == HdrMode::ToneMap, |this, _cx| this.settings.hdr = HdrMode::ToneMap, cx))
                                    .child(choice_chip("Passthrough", settings.hdr == HdrMode::Passthrough, |this, _cx| this.settings.hdr = HdrMode::Passthrough, cx))
                                    .child(choice_chip("Off", settings.hdr == HdrMode::Disabled, |this, _cx| this.settings.hdr = HdrMode::Disabled, cx)),
                            ))
                            .child(toggle_row(
                                "Debanding",
                                "Reduce visible color banding. May increase GPU workload.",
                                settings.deband,
                                |this, _cx| this.settings.deband = !this.settings.deband,
                                cx,
                            ))
                            .child(toggle_row(
                                "Frame interpolation",
                                "Optional motion smoothing. Disabled by default to avoid extra processing.",
                                settings.interpolation,
                                |this, _cx| this.settings.interpolation = !this.settings.interpolation,
                                cx,
                            )),
                    ))
                    .child(section(
                        "Subtitles",
                        "Set a comfortable baseline for subtitle readability. Per-track overrides can be added with the playback engine.",
                        div()
                            .flex()
                            .flex_col()
                            .gap_3()
                            .child(option_row(
                                "Font size",
                                "Choose the default subtitle font size.",
                                div()
                                    .flex()
                                    .flex_wrap()
                                    .gap_2()
                                    .child(choice_chip("Small · 32", settings.subtitles.font_size == 32, |this, _cx| this.settings.subtitles.font_size = 32, cx))
                                    .child(choice_chip("Default · 42", settings.subtitles.font_size == 42, |this, _cx| this.settings.subtitles.font_size = 42, cx))
                                    .child(choice_chip("Large · 52", settings.subtitles.font_size == 52, |this, _cx| this.settings.subtitles.font_size = 52, cx))
                                    .child(choice_chip("Extra large · 64", settings.subtitles.font_size == 64, |this, _cx| this.settings.subtitles.font_size = 64, cx)),
                            ))
                            .child(option_row(
                                "Vertical margin",
                                "Distance from the bottom edge, expressed as a percentage.",
                                div()
                                    .flex()
                                    .flex_wrap()
                                    .gap_2()
                                    .child(choice_chip("12%", settings.subtitles.vertical_margin == 12, |this, _cx| this.settings.subtitles.vertical_margin = 12, cx))
                                    .child(choice_chip("24%", settings.subtitles.vertical_margin == 24, |this, _cx| this.settings.subtitles.vertical_margin = 24, cx))
                                    .child(choice_chip("36%", settings.subtitles.vertical_margin == 36, |this, _cx| this.settings.subtitles.vertical_margin = 36, cx)),
                            ))
                            .child(toggle_row(
                                "Bold subtitles",
                                "Use a heavier subtitle weight for improved legibility.",
                                settings.subtitles.bold,
                                |this, _cx| this.settings.subtitles.bold = !this.settings.subtitles.bold,
                                cx,
                            )),
                    ))
                    .child(section(
                        "Buffering",
                        "Limits are normalized before being stored. These preferences are not yet connected to a streaming engine.",
                        div()
                            .flex()
                            .flex_col()
                            .gap_3()
                            .child(stepper_row(
                                "Forward buffer",
                                "Target seconds buffered ahead",
                                settings.buffer.forward_seconds,
                                "seconds",
                                |this, _cx| this.settings.buffer.forward_seconds = this.settings.buffer.forward_seconds.saturating_sub(5),
                                |this, _cx| this.settings.buffer.forward_seconds = this.settings.buffer.forward_seconds.saturating_add(5).min(300),
                                cx,
                            ))
                            .child(stepper_row(
                                "Back buffer",
                                "Retained media for backward seeking",
                                settings.buffer.back_seconds,
                                "seconds",
                                |this, _cx| this.settings.buffer.back_seconds = this.settings.buffer.back_seconds.saturating_sub(5),
                                |this, _cx| this.settings.buffer.back_seconds = this.settings.buffer.back_seconds.saturating_add(5).min(300),
                                cx,
                            ))
                            .child(stepper_row(
                                "Memory budget",
                                "Soft playback buffer memory limit",
                                settings.buffer.memory_limit_mib,
                                "MiB",
                                |this, _cx| this.settings.buffer.memory_limit_mib = this.settings.buffer.memory_limit_mib.saturating_sub(64).max(32),
                                |this, _cx| this.settings.buffer.memory_limit_mib = this.settings.buffer.memory_limit_mib.saturating_add(64).min(2048),
                                cx,
                            )),
                    ))
                    .child(section(
                        "Diagnostics",
                        "Copy a recent support report. Personal watch history and the local database are excluded.",
                        div()
                            .flex()
                            .items_center()
                            .justify_between()
                            .child(
                                div()
                                    .flex()
                                    .flex_col()
                                    .gap_1()
                                    .child(div().text_sm().text_color(rgb(0xE4E4E7)).child("Recent application logs"))
                                    .child(div().text_xs().text_color(rgb(0x858C94)).child("Includes up to 72 hours of diagnostic events.")),
                            )
                            .child(
                                div()
                                    .id("copy-diagnostics")
                                    .px_3()
                                    .py_2()
                                    .rounded_md()
                                    .bg(rgb(0x164E63))
                                    .text_color(rgb(0xCFFAFE))
                                    .text_sm()
                                    .cursor_pointer()
                                    .child("Copy logs")
                                    .on_click(cx.listener(|this, _event, _window, cx| {
                                        this.copy_diagnostics(cx);
                                    })),
                            ),
                    ))
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .justify_between()
                            .py_2()
                            .child(
                                div()
                                    .text_xs()
                                    .text_color(rgb(0x8B949E))
                                    .child(self.status.clone()),
                            )
                            .child(
                                div()
                                    .text_xs()
                                    .text_color(rgb(0x65707A))
                                    .child("Playback engine integration is planned"),
                            ),
                    ),
            )
    }
}

fn nav_item(label: &'static str, selected: bool) -> impl IntoElement {
    let background = if selected {
        rgb(0x1A3034)
    } else {
        rgb(0x101315)
    };
    let foreground = if selected {
        rgb(0x67E8F9)
    } else {
        rgb(0xA1A1AA)
    };

    div()
        .px_3()
        .py_2()
        .rounded_md()
        .bg(background)
        .text_sm()
        .text_color(foreground)
        .child(label)
}

fn toggle_row<F>(
    label: &'static str,
    description: &'static str,
    enabled: bool,
    action: F,
    cx: &mut Context<SenzoApp>,
) -> impl IntoElement
where
    F: Fn(&mut SenzoApp, &mut Context<SenzoApp>) + 'static,
{
    div()
        .flex()
        .items_center()
        .justify_between()
        .child(
            div()
                .flex()
                .flex_col()
                .gap_1()
                .child(div().text_sm().text_color(rgb(0xE4E4E7)).child(label))
                .child(div().text_xs().text_color(rgb(0x858C94)).child(description)),
        )
        .child(choice_chip(
            if enabled { "Enabled" } else { "Disabled" },
            enabled,
            action,
            cx,
        ))
}

fn stepper_row<Down, Up>(
    label: &'static str,
    description: &'static str,
    value: u16,
    unit: &'static str,
    decrement: Down,
    increment: Up,
    cx: &mut Context<SenzoApp>,
) -> impl IntoElement
where
    Down: Fn(&mut SenzoApp, &mut Context<SenzoApp>) + 'static,
    Up: Fn(&mut SenzoApp, &mut Context<SenzoApp>) + 'static,
{
    div()
        .flex()
        .items_center()
        .justify_between()
        .child(
            div()
                .flex()
                .flex_col()
                .gap_1()
                .child(div().text_sm().text_color(rgb(0xE4E4E7)).child(label))
                .child(div().text_xs().text_color(rgb(0x858C94)).child(description)),
        )
        .child(
            div()
                .flex()
                .items_center()
                .gap_2()
                .child(choice_chip("−", false, decrement, cx))
                .child(
                    div()
                        .w(px(88.))
                        .text_center()
                        .text_sm()
                        .text_color(rgb(0xE4E4E7))
                        .child(format!("{value} {unit}")),
                )
                .child(choice_chip("+", false, increment, cx)),
        )
}

fn main() {
    if let Err(error) = diagnostics::initialize() {
        eprintln!("SENZO diagnostics initialization failed: {error:#}");
    } else {
        diagnostics::record(
            diagnostics::Level::Audit,
            "application",
            "initializing local database",
        );
    }

    let database = match LocalDatabase::open_default() {
        Ok(database) => {
            diagnostics::record(
                diagnostics::Level::Audit,
                "storage",
                "local database opened successfully",
            );
            database
        }
        Err(error) => {
            diagnostics::record(
                diagnostics::Level::Error,
                "storage",
                &format!("failed to initialize local database: {error:#}"),
            );
            panic!("failed to initialize SENZO's local database: {error:#}");
        }
    };

    gpui_platform::application()
        .with_quit_mode(QuitMode::LastWindowClosed)
        .run({
            let mut database = Some(database);
            move |cx: &mut App| {
                let database = database.take().expect("SENZO window initialized once");
                cx.open_window(WindowOptions::default(), move |_window, cx| {
                    cx.new(move |_cx| SenzoApp::new(database))
                })
                .expect("failed to open SENZO window");

                cx.activate(true);
            }
        });
}
