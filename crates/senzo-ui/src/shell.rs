//! Native SENZO desktop shell: routes, focus, menus, dialogs and window actions.

use std::{collections::HashMap, sync::Arc, time::Duration};

use gpui::{
    App, Bounds, ColorExt, Context, FocusHandle, Image, ImageFormat, KeyBinding, Menu, MenuItem, MouseButton,
    NavigationDirection, QuitMode, Render, SharedString, TitlebarOptions, Window, WindowBounds, WindowOptions,
    actions, div, img, prelude::*, px, rgb, size,
};
use senzo_domain::AppPage;
use tokio::runtime::Handle;

use crate::{
    components::{divider, feature_tile, page_intro, status_panel, StatusTone},
    navigation::NavigationStack,
    theme::Theme,
};

actions!(senzo, [
    GoBack,
    GoForward,
    ToggleFullscreen,
    FocusNext,
    FocusPrevious,
    ActivateNavigation,
    MoveNavigationUp,
    MoveNavigationDown,
    ShowShortcuts,
    ShowAbout,
    ShowWhatsNew,
    Quit
]);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum DialogKind {
    About,
    Shortcuts,
}

pub(crate) struct SenzoShell {
    navigation: NavigationStack,
    navigation_focus: HashMap<AppPage, FocusHandle>,
    dialog_focus: FocusHandle,
    tokio: Handle,
    logo: Arc<Image>,
    dialog: Option<DialogKind>,
    menu_open: bool,
    notification: Option<String>,
    runtime_status: String,
    smoke_test: bool,
    smoke_test_scheduled: bool,
}

impl SenzoShell {
    pub(crate) fn new(tokio: Handle, smoke_test: bool, cx: &mut Context<Self>) -> Self {
        let navigation_focus = AppPage::ALL
            .iter()
            .enumerate()
            .map(|(index, page)| (*page, cx.focus_handle().tab_index(index as isize)))
            .collect();
        let dialog_focus = cx.focus_handle().tab_index(60);

        Self {
            navigation: NavigationStack::new(),
            navigation_focus,
            dialog_focus,
            tokio,
            logo: Arc::new(Image::from_bytes(
                ImageFormat::Png,
                include_bytes!("../../../assets/branding/senzo-logo.png").to_vec(),
            )),
            dialog: None,
            menu_open: false,
            notification: None,
            runtime_status: "Not checked yet".to_owned(),
            smoke_test,
            smoke_test_scheduled: false,
        }
    }

    fn focus_page(&self, page: AppPage, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(handle) = self.navigation_focus.get(&page) {
            window.focus(handle, cx);
        }
    }

    fn navigate(&mut self, page: AppPage, window: &mut Window, cx: &mut Context<Self>) {
        if self.navigation.navigate(page) {
            self.menu_open = false;
            self.focus_page(page, window, cx);
            cx.notify();
        }
    }

    fn move_nav_focus(&self, direction: isize, window: &mut Window, cx: &mut Context<Self>) {
        let current = AppPage::ALL
            .iter()
            .position(|page| {
                self.navigation_focus
                    .get(page)
                    .is_some_and(|handle| handle.is_focused(window))
            })
            .unwrap_or_else(|| {
                AppPage::ALL
                    .iter()
                    .position(|page| *page == self.navigation.current())
                    .unwrap_or(0)
            });
        let next = (current as isize + direction).rem_euclid(AppPage::ALL.len() as isize) as usize;
        self.focus_page(AppPage::ALL[next], window, cx);
    }

    fn go_back(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.dialog.take().is_some() {
            let page = self.navigation.current();
            self.focus_page(page, window, cx);
            cx.notify();
            return;
        }
        if self.menu_open {
            self.menu_open = false;
            let page = self.navigation.current();
            self.focus_page(page, window, cx);
            cx.notify();
            return;
        }
        if self.navigation.back() {
            let page = self.navigation.current();
            self.focus_page(page, window, cx);
            cx.notify();
        }
    }

    fn go_forward(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.dialog.is_some() || self.menu_open {
            return;
        }
        if self.navigation.can_go_forward() && self.navigation.forward() {
            let page = self.navigation.current();
            self.focus_page(page, window, cx);
            cx.notify();
        }
    }

    fn open_dialog(&mut self, kind: DialogKind, window: &mut Window, cx: &mut Context<Self>) {
        self.menu_open = false;
        self.dialog = Some(kind);
        window.focus(&self.dialog_focus, cx);
        cx.notify();
    }

    fn check_runtime(&mut self, cx: &mut Context<Self>) {
        let runtime = self.tokio.clone();
        self.runtime_status = "Running a background task…".to_owned();
        cx.notify();

        cx.spawn(async move |this, cx| {
            let result = runtime
                .spawn(async {
                    tokio::time::sleep(Duration::from_millis(120)).await;
                    "Background runtime is responding"
                })
                .await;
            let status = match result {
                Ok(message) => message.to_owned(),
                Err(error) => format!("Background task failed: {error}"),
            };
            let _ = this.update(cx, |this, cx| {
                this.runtime_status = status;
                cx.notify();
            });
        })
        .detach();
    }

    fn nav_item(&self, page: AppPage, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        let active = self.navigation.current() == page;
        let focus = self
            .navigation_focus
            .get(&page)
            .expect("every primary page has a focus handle")
            .clone();

        div()
            .id(format!("navigation-{}", page.key()))
            .track_focus(&focus)
            .tab_index(AppPage::ALL.iter().position(|item| *item == page).unwrap_or(0) as isize)
            .key_context("SENZO_NAVIGATION")
            .w_full()
            .flex()
            .items_center()
            .gap_3()
            .rounded_md()
            .border_1()
            .border_color(if active {
                rgb(Theme::ACCENT_DARK)
            } else {
                rgb(Theme::SIDEBAR)
            })
            .bg(if active {
                rgb(Theme::ACCENT_DARK)
            } else {
                rgb(Theme::SIDEBAR)
            })
            .px_3()
            .py_3()
            .cursor_pointer()
            .hover(|style| style.bg(rgb(Theme::SURFACE_HOVER)))
            .focus_visible(|style| {
                style
                    .border_1()
                    .border_color(rgb(Theme::ACCENT))
                    .bg(rgb(Theme::SURFACE_HOVER))
            })
            .aria_label(format!("{} navigation", page.title()))
            .aria_selected(active)
            .aria_keyshortcuts("Enter, Space")
            .on_click(cx.listener(move |this, _event, window, cx| {
                this.navigate(page, window, cx);
            }))
            .on_action(cx.listener(move |this, _: &ActivateNavigation, window, cx| {
                this.navigate(page, window, cx);
            }))
            .child(
                div()
                    .w(px(38.0))
                    .h(px(38.0))
                    .flex()
                    .items_center()
                    .justify_center()
                    .rounded_md()
                    .bg(if active {
                        rgb(Theme::ACCENT)
                    } else {
                        rgb(Theme::SURFACE_RAISED)
                    })
                    .text_color(if active {
                        rgb(Theme::BACKGROUND)
                    } else {
                        rgb(Theme::TEXT_SECONDARY)
                    })
                    .text_xs()
                    .child(page.short_label()),
            )
            .child(
                div()
                    .flex_1()
                    .text_sm()
                    .text_color(if active {
                        rgb(Theme::TEXT_PRIMARY)
                    } else {
                        rgb(Theme::TEXT_SECONDARY)
                    })
                    .child(page.title()),
            )
            .when(active, |element| {
                element.child(
                    div()
                        .w(px(5.0))
                        .h(px(5.0))
                        .rounded_full()
                        .bg(rgb(Theme::ACCENT)),
                )
            })
    }

    fn render_sidebar(&self, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        div()
            .id("senzo-sidebar")
            .key_context("SENZO_NAVIGATION")
            .w(px(Theme::SIDEBAR_WIDTH))
            .h_full()
            .flex()
            .flex_col()
            .gap_5()
            .bg(rgb(Theme::SIDEBAR))
            .border_r_1()
            .border_color(rgb(Theme::BORDER))
            .p_5()
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap_3()
                    .child(img(self.logo.clone()).size(px(44.0)))
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .gap_1()
                            .child(
                                div()
                                    .text_xl()
                                    .text_color(rgb(Theme::TEXT_PRIMARY))
                                    .child("SENZO"),
                            )
                            .child(
                                div()
                                    .text_xs()
                                    .text_color(rgb(Theme::TEXT_MUTED))
                                    .child("NATIVE MEDIA CENTER"),
                            ),
                    ),
            )
            .child(div().w_full().h(px(1.0)).bg(rgb(Theme::BORDER)))
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap_2()
                    .child(
                        div()
                            .px_2()
                            .text_xs()
                            .text_color(rgb(Theme::TEXT_MUTED))
                            .child("DISCOVER"),
                    )
                    .child(self.nav_item(AppPage::Home, cx))
                    .child(self.nav_item(AppPage::Movies, cx))
                    .child(self.nav_item(AppPage::TvShows, cx))
                    .child(self.nav_item(AppPage::Search, cx)),
            )
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap_2()
                    .child(
                        div()
                            .px_2()
                            .text_xs()
                            .text_color(rgb(Theme::TEXT_MUTED))
                            .child("YOUR SPACE"),
                    )
                    .child(self.nav_item(AppPage::Library, cx))
                    .child(self.nav_item(AppPage::Addons, cx)),
            )
            .child(div().flex_1())
            .child(self.nav_item(AppPage::Settings, cx))
            .child(
                div()
                    .rounded_md()
                    .border_1()
                    .border_color(rgb(Theme::BORDER))
                    .bg(rgb(Theme::SURFACE))
                    .p_3()
                    .flex()
                    .flex_col()
                    .gap_1()
                    .child(
                        div()
                            .text_xs()
                            .text_color(rgb(Theme::ACCENT))
                            .child("SENZO DESKTOP"),
                    )
                    .child(
                        div()
                            .text_xs()
                            .text_color(rgb(Theme::TEXT_MUTED))
                            .child("Rust · GPUI CE · Windows + Linux"),
                    ),
            )
    }

    fn render_header(&self, cx: &mut Context<Self>) -> gpui::Div {
        let page = self.navigation.current();
        div()
            .w_full()
            .h(px(Theme::HEADER_HEIGHT))
            .flex()
            .items_center()
            .justify_between()
            .border_b_1()
            .border_color(rgb(Theme::BORDER))
            .px_8()
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap_1()
                    .child(
                        div()
                            .text_2xl()
                            .text_color(rgb(Theme::TEXT_PRIMARY))
                            .child(page.title()),
                    )
                    .child(
                        div()
                            .text_xs()
                            .text_color(rgb(Theme::TEXT_MUTED))
                            .child(page.description()),
                    ),
            )
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap_2()
                    .child(
                        div()
                            .id("fullscreen-button")
                            .tab_index(20)
                            .rounded_md()
                            .border_1()
                            .border_color(rgb(Theme::BORDER))
                            .bg(rgb(Theme::SURFACE))
                            .px_3()
                            .py_2()
                            .cursor_pointer()
                            .hover(|style| style.bg(rgb(Theme::SURFACE_HOVER)))
                            .focus_visible(|style| style.border_1().border_color(rgb(Theme::ACCENT)))
                            .aria_label("Toggle fullscreen")
                            .aria_keyshortcuts("F11")
                            .on_click(|_event, window, _cx| window.toggle_fullscreen())
                            .child(
                                div()
                                    .text_xs()
                                    .text_color(rgb(Theme::TEXT_SECONDARY))
                                    .child("Fullscreen"),
                            ),
                    )
                    .child(
                        div()
                            .id("more-menu-button")
                            .tab_index(21)
                            .rounded_md()
                            .border_1()
                            .border_color(rgb(Theme::BORDER))
                            .bg(if self.menu_open {
                                rgb(Theme::ACCENT_DARK)
                            } else {
                                rgb(Theme::SURFACE)
                            })
                            .px_3()
                            .py_2()
                            .cursor_pointer()
                            .hover(|style| style.bg(rgb(Theme::SURFACE_HOVER)))
                            .focus_visible(|style| style.border_1().border_color(rgb(Theme::ACCENT)))
                            .aria_label("Open application menu")
                            .aria_expanded(self.menu_open)
                            .on_click(cx.listener(|this, _event, _window, cx| {
                                this.menu_open = !this.menu_open;
                                cx.notify();
                            }))
                            .child(
                                div()
                                    .text_xs()
                                    .text_color(rgb(Theme::TEXT_PRIMARY))
                                    .child("More  ···"),
                            ),
                    ),
            )
    }

    fn render_home(&self, cx: &mut Context<Self>) -> gpui::Div {
        div()
            .w_full()
            .flex()
            .flex_col()
            .gap_6()
            .child(
                div()
                    .w_full()
                    .rounded_lg()
                    .border_1()
                    .border_color(rgb(Theme::BORDER))
                    .bg(rgb(Theme::SURFACE))
                    .p_6()
                    .flex()
                    .flex_col()
                    .gap_4()
                    .child(
                        div()
                            .text_xs()
                            .text_color(rgb(Theme::ACCENT))
                            .child("YOUR NEXT FAVORITE STARTS HERE"),
                    )
                    .child(
                        div()
                            .text_2xl()
                            .text_color(rgb(Theme::TEXT_PRIMARY))
                            .child("Your entertainment. Your space."),
                    )
                    .child(
                        div()
                            .max_w(px(720.0))
                            .text_sm()
                            .text_color(rgb(Theme::TEXT_SECONDARY))
                            .child("SENZO is now running as a native desktop application. Discovery, artwork and catalogues will connect in the next development phase."),
                    )
                    .child(
                        div()
                            .flex()
                            .gap_3()
                            .child(
                                div()
                                    .id("home-explore-movies")
                                    .tab_index(30)
                                    .rounded_md()
                                    .bg(rgb(Theme::ACCENT))
                                    .px_4()
                                    .py_3()
                                    .cursor_pointer()
                                    .hover(|style| style.bg(rgb(0x67E8F9)))
                                    .focus_visible(|style| style.border_1().border_color(rgb(Theme::TEXT_PRIMARY)))
                                    .aria_label("Browse movies")
                                    .on_click(cx.listener(|this, _event, window, cx| {
                                        this.navigate(AppPage::Movies, window, cx);
                                    }))
                                    .child(
                                        div()
                                            .text_sm()
                                            .text_color(rgb(Theme::BACKGROUND))
                                            .child("Explore movies"),
                                    ),
                            )
                            .child(
                                div()
                                    .id("home-open-addons")
                                    .tab_index(31)
                                    .rounded_md()
                                    .border_1()
                                    .border_color(rgb(Theme::BORDER_STRONG))
                                    .bg(rgb(Theme::SURFACE_RAISED))
                                    .px_4()
                                    .py_3()
                                    .cursor_pointer()
                                    .hover(|style| style.bg(rgb(Theme::SURFACE_HOVER)))
                                    .focus_visible(|style| style.border_1().border_color(rgb(Theme::ACCENT)))
                                    .aria_label("Open add-ons")
                                    .on_click(cx.listener(|this, _event, window, cx| {
                                        this.navigate(AppPage::Addons, window, cx);
                                    }))
                                    .child(
                                        div()
                                            .text_sm()
                                            .text_color(rgb(Theme::TEXT_PRIMARY))
                                            .child("Manage add-ons"),
                                    ),
                            ),
                    ),
            )
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap_3()
                    .child(
                        div()
                            .text_lg()
                            .text_color(rgb(Theme::TEXT_PRIMARY))
                            .child("Built for your screen"),
                    )
                    .child(
                        div()
                            .flex()
                            .gap_3()
                            .child(feature_tile(
                                "DISCOVER",
                                "Movies & TV",
                                "A unified place for metadata, catalogues and recommendations.",
                            ))
                            .child(feature_tile(
                                "ORGANIZE",
                                "Your library",
                                "Keep saved titles, collections and viewing progress together.",
                            ))
                            .child(feature_tile(
                                "EXTEND",
                                "Your providers",
                                "Choose and manage compatible add-ons when you are ready.",
                            )),
                    ),
            )
            .child(status_panel(
                StatusTone::Info,
                "PHASE 2",
                "Native shell is ready",
                "This screen intentionally contains no fabricated movie recommendations. Real catalogues and artwork arrive with the metadata foundation.",
            ))
    }

    fn render_settings(&self, cx: &mut Context<Self>) -> gpui::Div {
        div()
            .w_full()
            .flex()
            .flex_col()
            .gap_5()
            .child(page_intro(
                "PREFERENCES",
                "Settings",
                "Everyday controls will stay simple, with advanced options kept separate.",
            ))
            .child(
                div()
                    .flex()
                    .gap_3()
                    .child(feature_tile(
                        "APPEARANCE",
                        "Display & appearance",
                        "Theme, window behavior and visual preferences.",
                    ))
                    .child(feature_tile(
                        "PLAYBACK",
                        "Playback",
                        "Player behavior, audio and subtitle preferences.",
                    ))
                    .child(feature_tile(
                        "LANGUAGE",
                        "Language",
                        "App language and preferred media languages.",
                    )),
            )
            .child(status_panel(
                if self.runtime_status.contains("failed") {
                    StatusTone::Error
                } else {
                    StatusTone::Info
                },
                "ADVANCED DIAGNOSTICS",
                "Background runtime",
                "A lightweight check confirms that the application-owned Tokio runtime can execute background work without blocking the GPUI interface.",
            ))
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap_3()
                    .rounded_md()
                    .border_1()
                    .border_color(rgb(Theme::BORDER))
                    .bg(rgb(Theme::SURFACE))
                    .p_4()
                    .child(
                        div()
                            .flex_1()
                            .text_sm()
                            .text_color(rgb(Theme::TEXT_SECONDARY))
                            .child(self.runtime_status.clone()),
                    )
                    .child(
                        div()
                            .id("runtime-check")
                            .tab_index(40)
                            .rounded_md()
                            .bg(rgb(Theme::ACCENT_DARK))
                            .px_3()
                            .py_2()
                            .cursor_pointer()
                            .focus_visible(|style| style.border_1().border_color(rgb(Theme::ACCENT)))
                            .aria_label("Run background runtime check")
                            .on_click(cx.listener(|this, _event, _window, cx| {
                                this.check_runtime(cx);
                            }))
                            .child(
                                div()
                                    .text_xs()
                                    .text_color(rgb(Theme::ACCENT))
                                    .child("Run check"),
                            ),
                    ),
            )
    }

    fn render_page(&self, cx: &mut Context<Self>) -> gpui::Div {
        match self.navigation.current() {
            AppPage::Home => self.render_home(cx),
            AppPage::Movies => div()
                .w_full()
                .flex()
                .flex_col()
                .gap_5()
                .child(page_intro(
                    "DISCOVER",
                    "Movies",
                    "Find films through trusted metadata and the catalogues you choose.",
                ))
                .child(status_panel(
                    StatusTone::Loading,
                    "COMING IN PHASE 3",
                    "Movie discovery is being connected",
                    "The native metadata client, catalogue requests and artwork pipeline are the next implementation milestone. No sample or invented titles are shown here.",
                )),
            AppPage::TvShows => div()
                .w_full()
                .flex()
                .flex_col()
                .gap_5()
                .child(page_intro(
                    "DISCOVER",
                    "TV Shows",
                    "Explore series, seasons and episodes in one native experience.",
                ))
                .child(status_panel(
                    StatusTone::Loading,
                    "COMING IN PHASE 3",
                    "TV discovery is being connected",
                    "Series metadata, season and episode details will populate this route when the native API client is implemented.",
                )),
            AppPage::Search => div()
                .w_full()
                .flex()
                .flex_col()
                .gap_5()
                .child(page_intro(
                    "FIND SOMETHING",
                    "Search",
                    "Search across built-in metadata and enabled providers.",
                ))
                .child(status_panel(
                    StatusTone::Info,
                    "SEARCH FOUNDATION",
                    "Search is not connected yet",
                    "Search input, query state and provider aggregation will arrive with the networking and metadata phases.",
                )),
            AppPage::Library => div()
                .w_full()
                .flex()
                .flex_col()
                .gap_5()
                .child(page_intro(
                    "YOUR SPACE",
                    "Library",
                    "Your saved titles and personal collections belong here.",
                ))
                .child(status_panel(
                    StatusTone::Empty,
                    "EMPTY LIBRARY",
                    "Your library is empty",
                    "When library and collection workflows are implemented, saved media will appear here. Your local data foundation is already in place.",
                )),
            AppPage::Addons => div()
                .w_full()
                .flex()
                .flex_col()
                .gap_5()
                .child(page_intro(
                    "EXTEND SENZO",
                    "Add-ons",
                    "Choose which external catalogues and stream providers SENZO can use.",
                ))
                .child(status_panel(
                    StatusTone::Empty,
                    "NO INSTALLED ADD-ONS",
                    "Nothing has been installed",
                    "Stremio-compatible add-on support arrives in Phase 6. Add-ons will require explicit user installation; Torrentio will never be bundled or enabled by default.",
                )),
            AppPage::Settings => self.render_settings(cx),
        }
    }

    fn render_menu(&self, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        div()
            .id("application-menu")
            .absolute()
            .top(px(Theme::HEADER_HEIGHT - 4.0))
            .right(px(28.0))
            .w(px(248.0))
            .flex()
            .flex_col()
            .gap_1()
            .rounded_lg()
            .border_1()
            .border_color(rgb(Theme::BORDER_STRONG))
            .bg(rgb(Theme::SURFACE_RAISED))
            .p_2()
            .aria_label("Application menu")
            .child(self.menu_item("Keyboard shortcuts", 50, cx.listener(|this, _event, window, cx| {
                this.open_dialog(DialogKind::Shortcuts, window, cx);
            })))
            .child(self.menu_item("About SENZO", 51, cx.listener(|this, _event, window, cx| {
                this.open_dialog(DialogKind::About, window, cx);
            })))
            .child(self.menu_item("What's new", 52, cx.listener(|this, _event, _window, cx| {
                this.menu_open = false;
                this.notification = Some("Phase 2 brings SENZO's native navigation, design system and desktop shell.".to_owned());
                cx.notify();
            })))
            .child(div().w_full().h(px(1.0)).bg(rgb(Theme::BORDER)))
            .child(self.menu_item("Quit SENZO", 53, cx.listener(|_, _event, _window, cx| cx.quit())))
    }

    fn menu_item<F>(
        &self,
        label: &'static str,
        tab_index: isize,
        on_click: F,
    ) -> impl IntoElement + use<F>
    where
        F: Fn(&gpui::ClickEvent, &mut Window, &mut App) + 'static,
    {
        div()
            .id(format!("menu-item-{label}"))
            .tab_index(tab_index)
            .w_full()
            .rounded_md()
            .px_3()
            .py_3()
            .cursor_pointer()
            .hover(|style| style.bg(rgb(Theme::SURFACE_HOVER)))
            .focus_visible(|style| style.border_1().border_color(rgb(Theme::ACCENT)))
            .aria_label(label)
            .aria_keyshortcuts("Enter")
            .on_click(on_click)
            .child(
                div()
                    .text_sm()
                    .text_color(rgb(Theme::TEXT_PRIMARY))
                    .child(label),
            )
    }

    fn render_dialog(&self, kind: DialogKind, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        let (eyebrow, title, description) = match kind {
            DialogKind::About => (
                "ABOUT",
                "SENZO",
                "A native desktop media center, re-engineered in Rust with GPUI CE.",
            ),
            DialogKind::Shortcuts => (
                "KEYBOARD",
                "Keyboard shortcuts",
                "Navigate SENZO without reaching for the mouse.",
            ),
        };

        let dialog = div()
            .id("dialog-overlay")
            .aria_label("SENZO dialog")
            .absolute()
            .inset_0()
            .flex()
            .items_center()
            .justify_center()
            .bg(rgb(0x000000).opacity(0.78))
            .child(
                div()
                    .w(px(540.0))
                    .rounded_lg()
                    .border_1()
                    .border_color(rgb(Theme::BORDER_STRONG))
                    .bg(rgb(Theme::SURFACE_RAISED))
                    .p_6()
                    .flex()
                    .flex_col()
                    .gap_4()
                    .child(
                        div()
                            .text_xs()
                            .text_color(rgb(Theme::ACCENT))
                            .child(eyebrow),
                    )
                    .child(
                        div()
                            .text_2xl()
                            .text_color(rgb(Theme::TEXT_PRIMARY))
                            .child(title),
                    )
                    .child(
                        div()
                            .text_sm()
                            .text_color(rgb(Theme::TEXT_SECONDARY))
                            .child(description),
                    ),
            );

        let details = match kind {
            DialogKind::About => vec![
                ("Version", "0.1.0 · development"),
                ("Technology", "Rust 2024 · GPUI CE 0.2.2"),
                ("Platforms", "Native Windows and Linux"),
                ("Playback", "MPV/libmpv integration is planned"),
            ],
            DialogKind::Shortcuts => vec![
                ("Back", "Escape or Alt + Left"),
                ("Forward", "Alt + Right"),
                ("Fullscreen", "F11"),
                ("Sidebar", "Up / Down, then Enter or Space"),
                ("Focus", "Tab / Shift + Tab"),
                ("Quit", "Ctrl + Q or Cmd + Q"),
            ],
        };

        let rows = details.into_iter().fold(
            div().flex().flex_col().gap_3(),
            |column, (label, value)| {
                column.child(
                    div()
                        .flex()
                        .items_center()
                        .justify_between()
                        .gap_4()
                        .child(
                            div()
                                .text_sm()
                                .text_color(rgb(Theme::TEXT_SECONDARY))
                                .child(label),
                        )
                        .child(
                            div()
                                .text_sm()
                                .text_color(rgb(Theme::TEXT_PRIMARY))
                                .child(value),
                        ),
                )
            },
        );

        dialog
            .child(rows)
            .child(divider())
            .child(
                div()
                    .flex()
                    .justify_end()
                    .child(
                        div()
                            .id("dialog-close")
                            .track_focus(&self.dialog_focus)
                            .tab_index(60)
                            .rounded_md()
                            .bg(rgb(Theme::ACCENT))
                            .px_4()
                            .py_2()
                            .cursor_pointer()
                            .focus_visible(|style| style.border_1().border_color(rgb(Theme::TEXT_PRIMARY)))
                            .aria_label("Close dialog")
                            .on_click(cx.listener(|this, _event, window, cx| {
                                this.dialog = None;
                                this.focus_page(this.navigation.current(), window, cx);
                                cx.notify();
                            }))
                            .child(
                                div()
                                    .text_sm()
                                    .text_color(rgb(Theme::BACKGROUND))
                                    .child("Close"),
                            ),
                    ),
            )
    }

    fn render_notification(&self, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        let message = self.notification.clone().unwrap_or_default();
        div()
            .id("notification-toast")
            .absolute()
            .bottom(px(24.0))
            .right(px(24.0))
            .max_w(px(420.0))
            .flex()
            .items_center()
            .gap_3()
            .rounded_md()
            .border_1()
            .border_color(rgb(Theme::BORDER_STRONG))
            .bg(rgb(Theme::SURFACE_RAISED))
            .p_4()
            .child(
                div()
                    .flex_1()
                    .text_sm()
                    .text_color(rgb(Theme::TEXT_PRIMARY))
                    .child(message),
            )
            .child(
                div()
                    .id("dismiss-notification")
                    .tab_index(70)
                    .rounded_md()
                    .px_2()
                    .py_1()
                    .cursor_pointer()
                    .hover(|style| style.bg(rgb(Theme::SURFACE_HOVER)))
                    .aria_label("Dismiss notification")
                    .on_click(cx.listener(|this, _event, _window, cx| {
                        this.notification = None;
                        cx.notify();
                    }))
                    .child(
                        div()
                            .text_xs()
                            .text_color(rgb(Theme::TEXT_SECONDARY))
                            .child("Dismiss"),
                    ),
            )
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
                    tracing::info!("native SENZO shell opened and rendered; ending smoke test");
                    cx.quit();
                });
            })
            .detach();
        }

        let sidebar = self.render_sidebar(cx);
        let header = self.render_header(cx);
        let page = self.render_page(cx);

        let mut root = div()
            .id("senzo-root")
            .key_context("SENZO_ROOT")
            .relative()
            .size_full()
            .flex()
            .bg(rgb(Theme::BACKGROUND))
            .text_color(rgb(Theme::TEXT_PRIMARY))
            .on_action(cx.listener(|this, _: &GoBack, window, cx| {
                this.go_back(window, cx);
            }))
            .on_action(cx.listener(|this, _: &GoForward, window, cx| {
                this.go_forward(window, cx);
            }))
            .on_action(|_: &ToggleFullscreen, window, _cx| {
                window.toggle_fullscreen();
            })
            .on_action(|_: &FocusNext, window, cx| {
                window.focus_next(cx);
            })
            .on_action(|_: &FocusPrevious, window, cx| {
                window.focus_prev(cx);
            })
            .on_action(cx.listener(|this, _: &MoveNavigationUp, window, cx| {
                this.move_nav_focus(-1, window, cx);
            }))
            .on_action(cx.listener(|this, _: &MoveNavigationDown, window, cx| {
                this.move_nav_focus(1, window, cx);
            }))
            .on_action(cx.listener(|this, _: &ShowShortcuts, window, cx| {
                this.open_dialog(DialogKind::Shortcuts, window, cx);
            }))
            .on_action(cx.listener(|this, _: &ShowAbout, window, cx| {
                this.open_dialog(DialogKind::About, window, cx);
            }))
            .on_action(cx.listener(|this, _: &ShowWhatsNew, _window, cx| {
                this.menu_open = false;
                this.notification = Some("Phase 2 brings SENZO's native navigation, design system and desktop shell.".to_owned());
                cx.notify();
            }))
            .on_action(|_: &Quit, _window, cx| cx.quit())
            .on_mouse_down(
                MouseButton::Navigate(NavigationDirection::Back),
                cx.listener(|this, _event, window, cx| this.go_back(window, cx)),
            )
            .on_mouse_down(
                MouseButton::Navigate(NavigationDirection::Forward),
                cx.listener(|this, _event, window, cx| this.go_forward(window, cx)),
            )
            .child(sidebar)
            .child(
                div()
                    .h_full()
                    .flex_1()
                    .flex()
                    .flex_col()
                    .child(header)
                    .child(
                        div()
                            .id("page-content")
                            .flex_1()
                            .overflow_y_scroll()
                            .p_8()
                            .child(page),
                    ),
            );

        if self.menu_open {
            root = root.child(self.render_menu(cx));
        }
        if let Some(kind) = self.dialog {
            root = root.child(self.render_dialog(kind, cx));
        }
        if self.notification.is_some() {
            root = root.child(self.render_notification(cx));
        }

        root
    }
}

fn bind_shell_keys(cx: &mut App) {
    cx.bind_keys([
        KeyBinding::new("escape", GoBack, Some("SENZO_ROOT")),
        KeyBinding::new("alt-left", GoBack, Some("SENZO_ROOT")),
        KeyBinding::new("alt-right", GoForward, Some("SENZO_ROOT")),
        KeyBinding::new("f11", ToggleFullscreen, Some("SENZO_ROOT")),
        KeyBinding::new("alt-enter", ToggleFullscreen, Some("SENZO_ROOT")),
        KeyBinding::new("tab", FocusNext, Some("SENZO_ROOT")),
        KeyBinding::new("shift-tab", FocusPrevious, Some("SENZO_ROOT")),
        KeyBinding::new("up", MoveNavigationUp, Some("SENZO_NAVIGATION")),
        KeyBinding::new("down", MoveNavigationDown, Some("SENZO_NAVIGATION")),
        KeyBinding::new("enter", ActivateNavigation, Some("SENZO_NAVIGATION")),
        KeyBinding::new("space", ActivateNavigation, Some("SENZO_NAVIGATION")),
        KeyBinding::new("cmd-q", Quit, None),
        KeyBinding::new("ctrl-q", Quit, None),
        KeyBinding::new("cmd-shift-/", ShowShortcuts, None),
    ]);
}

pub fn run(tokio: Handle, smoke_test: bool) {
    gpui_platform::application()
        .with_quit_mode(QuitMode::LastWindowClosed)
        .run(move |cx: &mut App| {
            bind_shell_keys(cx);

            cx.set_menus(vec![
                Menu {
                    name: "SENZO".into(),
                    disabled: false,
                    items: vec![
                        MenuItem::action("About SENZO", ShowAbout),
                        MenuItem::action("Keyboard shortcuts", ShowShortcuts),
                        MenuItem::action("Quit SENZO", Quit),
                    ],
                },
                Menu {
                    name: "Navigate".into(),
                    disabled: false,
                    items: vec![
                        MenuItem::action("Back", GoBack),
                        MenuItem::action("Forward", GoForward),
                    ],
                },
                Menu {
                    name: "View".into(),
                    disabled: false,
                    items: vec![MenuItem::action("Toggle Fullscreen", ToggleFullscreen)],
                },
            ]);

            let options = WindowOptions {
                window_bounds: Some(WindowBounds::Windowed(Bounds::centered(
                    None,
                    size(px(1280.0), px(720.0)),
                    cx,
                ))),
                titlebar: Some(TitlebarOptions {
                    title: Some(SharedString::from("SENZO")),
                    ..Default::default()
                }),
                app_id: Some("com.senzo.desktop".to_owned()),
                window_min_size: Some(size(
                    px(Theme::MIN_WINDOW_WIDTH),
                    px(Theme::MIN_WINDOW_HEIGHT),
                )),
                ..Default::default()
            };

            cx.open_window(options, move |window, cx| {
                let shell = cx.new(|cx| SenzoShell::new(tokio, smoke_test, cx));
                let initial_focus = shell
                    .read(cx)
                    .navigation_focus
                    .get(&AppPage::Home)
                    .expect("home navigation focus is initialized")
                    .clone();
                window.focus(&initial_focus, cx);
                shell
            })
            .expect("failed to open the SENZO native window");
            cx.activate(true);
        });
}


#[cfg(test)]
mod tests {
    use super::*;

    #[gpui::test]
    async fn keyboard_navigation_back_forward_and_focus_restoration(
        cx: &mut gpui::TestAppContext,
    ) {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("test Tokio runtime should initialize");

        let (shell, window_cx) = cx.add_window_view(|window, cx| {
            let shell = SenzoShell::new(runtime.handle().clone(), false, cx);
            let home_focus = shell
                .navigation_focus
                .get(&AppPage::Home)
                .expect("home focus handle exists")
                .clone();
            window.focus(&home_focus, cx);
            shell
        });

        window_cx.update(|_, app| bind_shell_keys(app));
        window_cx.simulate_keystrokes("down enter");
        assert_eq!(shell.read_with(window_cx, |shell, _| shell.navigation.current()), AppPage::Movies);
        assert!(window_cx.update(|window, app| {
            shell
                .read(app)
                .navigation_focus
                .get(&AppPage::Movies)
                .expect("movies focus handle exists")
                .is_focused(window)
        }));

        window_cx.simulate_keystrokes("escape");
        assert_eq!(shell.read_with(window_cx, |shell, _| shell.navigation.current()), AppPage::Home);
        window_cx.simulate_keystrokes("alt-right");
        assert_eq!(shell.read_with(window_cx, |shell, _| shell.navigation.current()), AppPage::Movies);

        window_cx.dispatch_action(ShowShortcuts);
        assert_eq!(
            shell.read_with(window_cx, |shell, _| shell.dialog),
            Some(DialogKind::Shortcuts)
        );
        window_cx.simulate_keystrokes("escape");
        assert_eq!(shell.read_with(window_cx, |shell, _| shell.dialog), None);
        assert!(window_cx.update(|window, app| {
            shell
                .read(app)
                .navigation_focus
                .get(&AppPage::Movies)
                .expect("movies focus handle exists")
                .is_focused(window)
        }));
    }
}
