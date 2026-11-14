            } else {
                div().w_full().min_h(px(220.)).rounded_xl().bg(rgb(0x151D20)).p_6().flex().flex_col().justify_between()
                    .child(div().text_xs().text_color(rgb(0x67E8F9)).child("WELCOME TO SENZO"))
                    .child(div().flex().flex_col().gap_2()
                        .child(div().text_2xl().font_weight(gpui::FontWeight::BOLD).child("Your next favourite starts here"))
                        .child(div().text_sm().text_color(rgb(0xA1A1AA)).child(home_catalog_status.clone())))
            };
            this.child(hero).child(div().text_xs().text_color(rgb(0x858C94)).child(home_catalog_status.clone()))
                .when(!continue_watching_cards.is_empty(), |panel| {
                    panel.child(div().flex().flex_col().gap_3()
                        .child(div().text_lg().font_weight(gpui::FontWeight::SEMIBOLD).child("Continue watching"))
                        .child(div().w_full().id("continue-watching-shelf").overflow_x_scroll().flex().gap_3().children(continue_watching_cards)))
                })
                .children(["Trending this week", "Popular movies", "Popular TV"].into_iter().filter_map(|shelf_name| {
                    let entries = home_catalogs.iter().filter(|(name, _, _)| name == shelf_name).cloned().collect::<Vec<_>>();
                    if entries.is_empty() { return None; }
                    let cards = entries.into_iter().map(|(_, kind, item)| {
                        let title = item.display_title().unwrap_or("Untitled").to_owned();
                        let year = item.release_date.as_deref().or(item.first_air_date.as_deref())
                            .and_then(|date| date.get(0..4)).unwrap_or("—").to_owned();
                        let rating = item.vote_average.map(|value| format!("★ {value:.1}")).unwrap_or_default();
                        let selected = item.clone();
                        let selected_kind = kind.clone();
                        let poster = tmdb_artwork_url(item.poster_path.as_deref(), "w342");
                        div().w(px(164.)).flex_shrink_0().rounded_lg().bg(rgb(0x171D20))
                            .border_1().border_color(rgb(0x263238)).overflow_hidden().flex().flex_col().gap_2()
                            .child(div().w_full().h(px(236.)).bg(rgb(0x20272B)).when_some(poster, |panel, url| {
                                panel.child(img(url).w_full().h(px(236.)).object_fit(ObjectFit::Cover))
                            }))
                            .child(div().px_2().pb_2().flex().flex_col().gap_1()
                                .child(div().text_sm().font_weight(gpui::FontWeight::SEMIBOLD).child(title))
                                .child(div().text_xs().text_color(rgb(0x9299A1)).child(format!("{year} · {rating}")))
                                .child(dynamic_choice_chip("Details".into(), false, move |this, cx| this.select_tmdb_title(selected_kind.clone(), selected.clone(), cx), cx)))
                    }).collect::<Vec<_>>();
                    Some(div().flex().flex_col().gap_3()
                        .child(div().flex().items_center().justify_between()
                            .child(div().text_lg().font_weight(gpui::FontWeight::SEMIBOLD).child(shelf_name))
                            .child(dynamic_choice_chip("View all".into(), false, move |this, cx| {
                                this.load_tmdb_catalog(if shelf_name == "Popular TV" { "tv" } else { "movie" }, None, cx);
                            }, cx)))
                        .child(div().w_full().id(shelf_name).overflow_x_scroll().flex().gap_3().children(cards)))
                }))
                .children(home_addon_catalogs.iter().map(|(shelf_name, addon, media_type, items)| {
                    let cards = items.iter().filter_map(|item| {
                        let title = item.get("name").or_else(|| item.get("title"))
                            .and_then(Value::as_str)?.to_owned();
                        let selected = item.clone();
                        let selected_addon = addon.clone();
                        let poster = safe_addon_artwork_url(item.get("poster").and_then(Value::as_str));
                        let year = item.get("releaseInfo").and_then(Value::as_str)
                            .or_else(|| item.get("year").and_then(Value::as_str))
                            .unwrap_or(" ");
                        Some(div().w(px(164.)).flex_shrink_0().rounded_lg().bg(rgb(0x171D20))
                            .border_1().border_color(rgb(0x263238)).overflow_hidden().flex().flex_col().gap_2()
                            .child(div().w_full().h(px(236.)).bg(rgb(0x20272B))
                                .when_some(poster, |panel, url| {
                                    panel.child(img(url).w_full().h(px(236.)).object_fit(ObjectFit::Cover))
                                }))
                            .child(div().px_2().pb_2().flex().flex_col().gap_1()
                                .child(div().text_sm().font_weight(gpui::FontWeight::SEMIBOLD).child(title))
                                .child(div().text_xs().text_color(rgb(0x9299A1)).child(year.to_owned()))
                                .child(dynamic_choice_chip("Details".into(), false, move |this, cx| {
                                    this.select_addon_catalog_meta(selected_addon.clone(), selected.clone(), cx)
                                }, cx)))
                        )
                    }).collect::<Vec<_>>();
                    div().flex().flex_col().gap_3()
                        .child(div().flex().items_center().justify_between()
                            .child(div().text_lg().font_weight(gpui::FontWeight::SEMIBOLD).child(shelf_name.clone()))
                            .child(div().text_xs().text_color(rgb(0x67E8F9)).child("ADD-ON")))
                        .child(div().w_full().id(format!("addon-shelf-{}-{}", addon.id, media_type))
                            .overflow_x_scroll().flex().gap_3().children(cards))
                }))
        })
        .when(is_details, |this| {
            this.child(
                div()
                    .flex()
                    .gap_6()
                    .p_5()
                    .rounded_xl()
                    .bg(rgb(0x151719))
                    .child(
                        div()
                            .w(px(190.))
                            .h(px(285.))