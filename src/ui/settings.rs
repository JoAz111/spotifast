//! The Settings page.

use egui::{Align, CornerRadius, Frame, Layout, Margin, Stroke, Vec2};

use crate::api::models::pick_image;
use crate::app::App;
use crate::i18n::{gettext, pgettext};
use crate::model::{Action, Dialog};
use crate::settings::{LanguageChoice, ProxyMode};
use crate::theme::{self, Icon, Palette};

use super::widgets;

const PLAYBACK_DIRTY_ID: &str = "playback-settings-dirty";
const SETTINGS_FILTER_ID: &str = "settings-filter";

/// Whether a settings row matches the filter query (case-insensitive).
/// An empty query matches everything, so the page reads exactly as
/// before when the search field is empty.
pub(crate) fn row_matches(needle: &str, title: &str, description: &str) -> bool {
    let needle = needle.trim().to_lowercase();
    if needle.is_empty() {
        return true;
    }
    title.to_lowercase().contains(&needle) || description.to_lowercase().contains(&needle)
}

/// Text shared by section selection and the rendered row. Conditional rows
/// participate only on the platforms and account states where they exist.
struct RowText<'a> {
    title: std::borrow::Cow<'a, str>,
    description: std::borrow::Cow<'a, str>,
    available: bool,
}

impl<'a> RowText<'a> {
    fn new(
        title: impl Into<std::borrow::Cow<'a, str>>,
        description: impl Into<std::borrow::Cow<'a, str>>,
    ) -> Self {
        Self {
            title: title.into(),
            description: description.into(),
            available: true,
        }
    }

    fn when(mut self, available: bool) -> Self {
        self.available = available;
        self
    }

    fn matches(&self, needle: &str, section: &str) -> bool {
        self.available
            && (needle.is_empty()
                || section.to_lowercase().contains(needle)
                || row_matches(needle, &self.title, &self.description))
    }
}

fn section_matches(needle: &str, title: &str, rows: &[RowText<'_>]) -> bool {
    let needle = needle.trim().to_lowercase();
    rows.iter().any(|row| row.matches(&needle, title))
}

fn filtered_row(
    ui: &mut egui::Ui,
    palette: &Palette,
    needle: &str,
    section: &str,
    row: &RowText<'_>,
    control: impl FnOnce(&mut egui::Ui),
) {
    if row.matches(needle, section) {
        widgets::setting_row(ui, palette, &row.title, &row.description, control);
    }
}

/// `filtered_row` for a control that needs `control_width` points.
fn filtered_row_sized(
    ui: &mut egui::Ui,
    palette: &Palette,
    needle: &str,
    section: &str,
    row: &RowText<'_>,
    control_width: f32,
    control: impl FnOnce(&mut egui::Ui),
) {
    if row.matches(needle, section) {
        widgets::setting_row_sized(
            ui,
            palette,
            &row.title,
            &row.description,
            control_width,
            control,
        );
    }
}

const PROXY_DIRTY_ID: &str = "proxy-settings-dirty";

fn section(
    ui: &mut egui::Ui,
    palette: &Palette,
    title: &str,
    add_contents: impl FnOnce(&mut egui::Ui),
) {
    ui.add_space(10.0);
    theme::text(ui, title, theme::bold(18.0), palette.text);
    ui.add_space(8.0);
    Frame::new()
        .fill(palette.surface.gamma_multiply(0.7))
        .stroke(Stroke::new(1.0, palette.outline))
        .corner_radius(CornerRadius::same(theme::RADIUS + 2))
        .inner_margin(Margin::symmetric(20, 16))
        .show(ui, |ui| {
            ui.set_width(ui.available_width().min(760.0));
            add_contents(ui);
        });
    ui.add_space(8.0);
}

pub fn show(app: &mut App, ui: &mut egui::Ui) {
    let palette = app.palette;
    let locale = app.locale;
    ui.add_space(8.0);
    theme::text(
        ui,
        gettext(locale, "Settings"),
        theme::bold(28.0),
        palette.text,
    );
    ui.add_space(4.0);
    let mut filter = ui
        .data(|data| data.get_temp::<String>(egui::Id::new(SETTINGS_FILTER_ID)))
        .unwrap_or_default();
    widgets::search_field(
        ui,
        &palette,
        app.locale,
        egui::Id::new("settings-search-field"),
        &mut filter,
        &gettext(locale, "Search settings"),
        ui.available_width().min(400.0),
    );
    let needle = filter.trim().to_lowercase();
    ui.data_mut(|data| data.insert_temp(egui::Id::new(SETTINGS_FILTER_ID), filter));
    ui.add_space(4.0);
    let dirty_id = egui::Id::new(PLAYBACK_DIRTY_ID);
    let mut playback_dirty = ui
        .data(|data| data.get_temp::<bool>(dirty_id))
        .unwrap_or(false);
    let proxy_dirty_id = egui::Id::new(PROXY_DIRTY_ID);
    let mut proxy_dirty = ui
        .data(|data| data.get_temp::<bool>(proxy_dirty_id))
        .unwrap_or(false);
    let mut changed = false;
    let mut any_visible = false;

    let account = gettext(locale, "Account");
    let sign_out = gettext(locale, "Sign out");
    let account_rows = [RowText::new(sign_out.clone(), account.clone())];
    if section_matches(&needle, &account, &account_rows) {
        any_visible = true;
        section(ui, &palette, &account, |ui| {
            if app.soloist_mode {
                ui.label("Sign out requests a pause. Soloist pairing stays saved. To change Spotify accounts, open the app settings first.");
            }
            if row_matches(&needle, &format!("{sign_out} {account}"), "") {
                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing.x = 14.0;
                    let avatar = app
                        .user
                        .as_ref()
                        .and_then(|user| pick_image(&user.images, 64).map(str::to_string));
                    widgets::cover(ui, &palette, avatar.as_deref(), 56.0, 28.0, Icon::User);
                    ui.vertical(|ui| {
                        let name = app
                            .user
                            .as_ref()
                            .map(|user| user.name().to_string())
                            .unwrap_or_default();
                        theme::text(ui, name, theme::semibold(16.0), palette.text);
                        let product = app
                            .user
                            .as_ref()
                            .and_then(|user| user.product.clone())
                            .map(|product| match product.as_str() {
                                "premium" => "Spotify Premium".to_string(),
                                "free" | "open" => {
                                    gettext(locale, "Spotify Free, local playback needs Premium")
                                        .into_owned()
                                }
                                other => other.to_string(),
                            })
                            .unwrap_or_default();
                        theme::text(ui, product, theme::regular(13.0), palette.secondary);
                        if let Some(username) =
                            app.local.connected.then(|| app.local.username.clone())
                            && !username.is_empty()
                        {
                            theme::text(
                                ui,
                                // Translators: {username} is the Spotify account's user name.
                                gettext(locale, "Connected as {username}")
                                    .replace("{username}", &username),
                                theme::regular(12.0),
                                palette.dim,
                            );
                        }
                    });
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        if theme::pill_button(ui, &palette, &sign_out, false).clicked() {
                            app.actions.push(Action::SignOut);
                        }
                    });
                });
                ui.add_space(10.0);
            }
        });
    }

    let (status, mut detail, mut action) = match &app.local_playback {
        crate::backend::LocalPlayback::Ready { .. } => (
            pgettext(locale, "playback status", "Ready"),
            gettext(locale, "This computer is a Spotify Connect device."),
            None,
        ),
        crate::backend::LocalPlayback::Authorizing => (
            pgettext(locale, "playback status", "Setting up"),
            gettext(locale, "Finish authorizing in your browser."),
            None,
        ),
        crate::backend::LocalPlayback::Connecting => (
            pgettext(locale, "playback status", "Connecting"),
            gettext(locale, "Connecting to Spotify…"),
            None,
        ),
        crate::backend::LocalPlayback::WaitingForNetwork => (
            pgettext(locale, "playback status", "Waiting for network"),
            gettext(
                locale,
                "Waiting for network connectivity to resume playback.",
            ),
            None,
        ),
        crate::backend::LocalPlayback::Failed(message) => (
            pgettext(locale, "playback status", "Unavailable"),
            message.clone().into(),
            Some(gettext(locale, "Try again")),
        ),
        crate::backend::LocalPlayback::Unavailable => (
            pgettext(locale, "playback status", "Not set up"),
            gettext(
                locale,
                "Requires Spotify Premium and a one-time browser sign-in.",
            ),
            Some(gettext(locale, "Enable playback here")),
        ),
    };
    if app.soloist_mode {
        detail = if app.local_ready {
            gettext(
                locale,
                "Soloist is connected. Audio quality is controlled by the receiver.",
            )
        } else {
            gettext(
                locale,
                "Open Spotifast Soloist to finish connecting your account.",
            )
        };
        action = None;
    }
    let playback = gettext(locale, "Playback on this computer");
    let normalize_volume = gettext(locale, "Normalize volume");
    let autoplay = gettext(locale, "Autoplay");
    let gapless = gettext(locale, "Gapless playback");
    let keep_playing = gettext(locale, "Keep music playing when the window closes");
    let update_checks = gettext(locale, "Automatic update checks");
    let audio_cache = gettext(locale, "Audio cache");
    let apply_playback = gettext(locale, "Apply and restart playback");
    let apply_playback_note = gettext(locale, "Restart local playback to apply these settings.");
    let download_updates = gettext(locale, "Download updates automatically");
    let playback_rows = [
        RowText::new(
            // Translators: {status} is a playback state such as Ready or Not set up.
            gettext(locale, "Status: {status}").replace("{status}", &status),
            detail,
        ),
        RowText::new(
            gettext(locale, "Device name"),
            gettext(locale, "How this computer appears in Spotify Connect."),
        ).when(!app.soloist_mode),
        RowText::new(
            gettext(locale, "Audio quality"),
            gettext(locale, "Higher bitrates use more data and cache space."),
        ).when(!app.soloist_mode),
        RowText::new(
            normalize_volume.clone(),
            gettext(locale, "Keep loud and quiet tracks at a similar level."),
        ).when(!app.soloist_mode),
        RowText::new(
            autoplay.clone(),
            gettext(locale, "Keep playing similar songs when your music ends."),
        ).when(!app.soloist_mode),
        RowText::new(
            gapless.clone(),
            gettext(locale, "Play tracks without silence between them."),
        ).when(!app.soloist_mode),
        RowText::new(
            keep_playing.clone(),
            super::keys::platform_shortcut(
                &gettext(
                    locale,
                    "Spotifast hides to the system tray. Quit from the tray menu or with Ctrl+Q.",
                ),
                &gettext(
                    locale,
                    "Spotifast hides to the system tray. Quit from the tray menu or with Cmd+Q.",
                ),
            )
            .to_owned(),
        ),
        RowText::new(
            update_checks.clone(),
            gettext(locale, "Checks GitHub once a day. No personal data is sent."),
        ),
        RowText::new(
            gettext(locale, "Audio output"),
            gettext(
                locale,
                "PulseAudio also covers PipeWire. Rodio talks to ALSA directly.",
            ),
        )
        .when(cfg!(target_os = "linux") && !app.soloist_mode),
        RowText::new(
            gettext(locale, "Output buffer"),
            gettext(
                locale,
                "More buffering can prevent clicks on busy computers. Less buffering makes controls respond sooner.",
            ),
        )
        .when(cfg!(windows) && !app.soloist_mode),
        RowText::new(
            audio_cache.clone(),
            gettext(locale, "Save downloaded audio for later playback."),
        ).when(!app.soloist_mode),
        RowText::new(apply_playback.clone(), apply_playback_note.clone()).when(playback_dirty && !app.soloist_mode),
        RowText::new(gettext(locale, "Playback settings applied"), "").when(!playback_dirty && !app.soloist_mode),
        RowText::new(
            download_updates.clone(),
            gettext(
                locale,
                "Downloads in the background. You choose when to restart.",
            ),
        ),
        RowText::new(
            gettext(locale, "MacBook notch widget"),
            gettext(
                locale,
                "Show interactive Now Playing widget when hovering over the notch.",
            ),
        )
        .when(cfg!(target_os = "macos")),
        RowText::new(
            gettext(locale, "Audio output"),
            gettext(locale, "Choose the speaker or headphones for this Mac."),
        ).when(cfg!(target_os = "macos") && app.soloist_mode),
    ];
    if section_matches(&needle, &playback, &playback_rows) {
        any_visible = true;
        section(ui, &palette, &playback, |ui| {
            if app.soloist_mode {
                widgets::setting_row(
                    ui,
                    &palette,
                    "Playlist and lyrics access",
                    "An optional Spotify browser approval enables playlist folders, lyrics and radio.",
                    |ui| {
                        if app.music_session_ready {
                            ui.label("Connected");
                        } else if app.music_session_busy {
                            ui.label("Connecting…");
                        } else if theme::pill_button(ui, &palette, "Enable", true).clicked() {
                            app.actions.push(Action::EnablePlayback);
                        }
                    },
                );
            }
            filtered_row(ui, &palette, &needle, &playback, &playback_rows[15], |ui| {
                if theme::pill_button(ui, &palette, &gettext(locale, "Audio output…"), false)
                    .clicked()
                {
                    app.actions
                        .push(Action::OpenUrl("spotifast-soloist://settings".into()));
                }
            });
            filtered_row(ui, &palette, &needle, &playback, &playback_rows[0], |ui| {
                if let Some(label) = action {
                    if theme::pill_button(ui, &palette, &label, true).clicked() {
                        app.actions.push(Action::EnablePlayback);
                    }
                } else if app.local_ready
                    && theme::soft_button(
                        ui,
                        &palette,
                        Some(Icon::Refresh),
                        &gettext(locale, "Reconnect"),
                        false,
                    )
                    .clicked()
                {
                    app.actions.push(Action::RestartEngine);
                }
            });
            filtered_row(ui, &palette, &needle, &playback, &playback_rows[1], |ui| {
                let response = Frame::new()
                    .fill(palette.surface)
                    .corner_radius(CornerRadius::same(6))
                    .inner_margin(Margin::symmetric(10, 6))
                    .show(ui, |ui| {
                        widgets::text_edit(
                            ui,
                            locale,
                            egui::TextEdit::singleline(&mut app.settings.device_name)
                                .font(theme::regular(14.0))
                                .frame(egui::Frame::NONE)
                                .desired_width(200.0),
                        )
                    })
                    .inner;
                if response.changed() {
                    changed = true;
                    playback_dirty = true;
                }
            });
            // Normal to Very high, left to right when side by side and top
            // to bottom in a column.
            let choices = [
                (96u16, gettext(locale, "Normal · 96 kbps")),
                (160, gettext(locale, "High · 160 kbps")),
                (320, gettext(locale, "Very high · 320 kbps")),
            ];
            let choice_gap = 6.0;
            let choices_width = choices
                .iter()
                .map(|(_, label)| theme::soft_button_width(ui, label))
                .sum::<f32>()
                + choice_gap * (choices.len() - 1) as f32;
            filtered_row_sized(
                ui,
                &palette,
                &needle,
                &playback,
                &playback_rows[2],
                choices_width,
                |ui| {
                    let mut choose = |ui: &mut egui::Ui, kbps: u16, label: &str| {
                        if theme::soft_button(
                            ui,
                            &palette,
                            None,
                            label,
                            app.settings.bitrate == kbps,
                        )
                        .clicked()
                            && app.settings.bitrate != kbps
                        {
                            app.settings.bitrate = kbps;
                            changed = true;
                            playback_dirty = true;
                        }
                    };
                    if ui.available_width() >= choices_width {
                        // Laid right to left, so the last choice goes first.
                        ui.horizontal(|ui| {
                            ui.spacing_mut().item_spacing.x = choice_gap;
                            for (kbps, label) in choices.iter().rev() {
                                choose(ui, *kbps, label);
                            }
                        });
                    } else {
                        // Too narrow even for a line of their own: a column.
                        ui.with_layout(Layout::top_down(Align::Max), |ui| {
                            ui.spacing_mut().item_spacing.y = choice_gap;
                            for (kbps, label) in &choices {
                                choose(ui, *kbps, label);
                            }
                        });
                    }
                },
            );
            filtered_row(ui, &palette, &needle, &playback, &playback_rows[3], |ui| {
                if widgets::switch(
                    ui,
                    &palette,
                    &normalize_volume,
                    &mut app.settings.normalisation,
                )
                .changed()
                {
                    changed = true;
                    playback_dirty = true;
                }
            });
            filtered_row(ui, &palette, &needle, &playback, &playback_rows[4], |ui| {
                if widgets::switch(ui, &palette, &autoplay, &mut app.settings.autoplay).changed() {
                    changed = true;
                    playback_dirty = true;
                }
            });
            filtered_row(ui, &palette, &needle, &playback, &playback_rows[5], |ui| {
                if widgets::switch(ui, &palette, &gapless, &mut app.settings.gapless).changed() {
                    changed = true;
                    playback_dirty = true;
                }
            });
            filtered_row(ui, &palette, &needle, &playback, &playback_rows[6], |ui| {
                if widgets::switch(
                    ui,
                    &palette,
                    &keep_playing,
                    &mut app.settings.keep_playing_in_background,
                )
                .changed()
                {
                    changed = true;
                }
            });
            #[cfg(target_os = "macos")]
            filtered_row(ui, &palette, &needle, &playback, &playback_rows[14], |ui| {
                if widgets::switch(
                    ui,
                    &palette,
                    &gettext(locale, "MacBook notch widget"),
                    &mut app.settings.mac_notch_widget,
                )
                .changed()
                {
                    changed = true;
                }
            });
            filtered_row(ui, &palette, &needle, &playback, &playback_rows[7], |ui| {
                if widgets::switch(
                    ui,
                    &palette,
                    &update_checks,
                    &mut app.settings.check_for_updates,
                )
                .changed()
                {
                    changed = true;
                }
            });
            filtered_row(ui, &palette, &needle, &playback, &playback_rows[13], |ui| {
                if widgets::switch(
                    ui,
                    &palette,
                    &download_updates,
                    &mut app.settings.download_updates_automatically,
                )
                .changed()
                {
                    changed = true;
                }
            });
            if cfg!(target_os = "linux") {
                filtered_row(ui, &palette, &needle, &playback, &playback_rows[8], |ui| {
                    let current = app
                        .settings
                        .platform_backend()
                        .unwrap_or_else(|| "rodio".into());
                    ui.horizontal(|ui| {
                        ui.spacing_mut().item_spacing.x = 6.0;
                        for backend in ["rodio", "pulseaudio"] {
                            let label = if backend == "pulseaudio" {
                                "PulseAudio / PipeWire"
                            } else {
                                "ALSA (rodio)"
                            };
                            if theme::soft_button(ui, &palette, None, label, current == backend)
                                .clicked()
                                && current != backend
                            {
                                app.settings.audio_backend = Some(backend.to_string());
                                changed = true;
                                playback_dirty = true;
                            }
                        }
                    });
                });
            }
            #[cfg(windows)]
            filtered_row(ui, &palette, &needle, &playback, &playback_rows[9], |ui| {
                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing.x = 6.0;
                    let current = app.settings.audio_buffer_ms;
                    for ms in [50u32, 100, 200] {
                        let label = format!("{ms} ms");
                        if theme::soft_button(ui, &palette, None, &label, current == ms).clicked()
                            && current != ms
                        {
                            app.settings.audio_buffer_ms = ms;
                            changed = true;
                            playback_dirty = true;
                        }
                    }
                });
            });
            filtered_row(ui, &palette, &needle, &playback, &playback_rows[10], |ui| {
                // The control area lays out right-to-left: add the rightmost item first.
                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing.x = 6.0;
                    if widgets::switch(ui, &palette, &audio_cache, &mut app.settings.audio_cache)
                        .changed()
                    {
                        changed = true;
                        playback_dirty = true;
                    }
                    if app.settings.audio_cache {
                        ui.add_space(6.0);
                        for (mb, label) in [(4096u64, "4 GB"), (1024, "1 GB"), (512, "512 MB")] {
                            if theme::soft_button(
                                ui,
                                &palette,
                                None,
                                label,
                                app.settings.audio_cache_mb == mb,
                            )
                            .clicked()
                                && app.settings.audio_cache_mb != mb
                            {
                                app.settings.audio_cache_mb = mb;
                                changed = true;
                                playback_dirty = true;
                            }
                        }
                    }
                });
            });
            ui.add_space(4.0);
            if playback_dirty
                || playback_rows[11].matches(&needle, &playback)
                || playback_rows[12].matches(&needle, &playback)
            {
                ui.horizontal(|ui| {
                    if playback_dirty {
                        if theme::pill_button(ui, &palette, &apply_playback, true).clicked() {
                            app.actions.push(Action::RestartEngine);
                            playback_dirty = false;
                        }
                        theme::subtle(ui, &palette, &apply_playback_note);
                    } else {
                        theme::subtle(ui, &palette, &gettext(locale, "Playback settings applied."));
                    }
                });
            }
        });
    }

    let appearance = gettext(locale, "Appearance");
    let sidebar_compact = gettext(locale, "Compact library sidebar");
    let tracklist_compact = gettext(locale, "Compact track list");
    let middle_click = gettext(locale, "Middle-click autoscroll");
    let custom_titlebar = gettext(locale, "Custom title bar");
    let show_custom_titlebar = cfg!(target_os = "linux") || app.windows_controls_visible();
    let appearance_rows = [
        RowText::new(
            gettext(locale, "Language"),
            gettext(
                locale,
                "System follows your computer's language. Untranslated text stays in English.",
            ),
        ),
        RowText::new(
            sidebar_compact.clone(),
            gettext(locale, "Show names without covers in the sidebar."),
        ),
        RowText::new(
            tracklist_compact.clone(),
            gettext(locale, "Show each track on one line without a cover."),
        ),
        RowText::new(
            gettext(locale, "Interface zoom"),
            super::keys::platform_shortcut(
                &gettext(
                    locale,
                    "Ctrl+Plus and Ctrl+Minus work anywhere; Ctrl+0 resets.",
                ),
                &gettext(
                    locale,
                    "Cmd+Plus and Cmd+Minus work anywhere; Cmd+0 resets.",
                ),
            )
            .to_owned(),
        ),
        RowText::new(
            middle_click.clone(),
            gettext(
                locale,
                "Middle-click a list, then move the pointer to scroll it. Off by default, because a middle click usually pastes on Linux.",
            ),
        )
        .when(cfg!(target_os = "linux")),
        RowText::new(
            custom_titlebar.clone(),
            gettext(
                locale,
                "Draw Spotifast's own title bar and window buttons instead of the desktop's.",
            ),
        )
        .when(show_custom_titlebar),
    ];
    if section_matches(&needle, &appearance, &appearance_rows) {
        any_visible = true;
        section(ui, &palette, &appearance, |ui| {
            filtered_row(
                ui,
                &palette,
                &needle,
                &appearance,
                &appearance_rows[0],
                |ui| language_picker(app, ui),
            );
            filtered_row(
                ui,
                &palette,
                &needle,
                &appearance,
                &appearance_rows[1],
                |ui| {
                    if widgets::switch(
                        ui,
                        &palette,
                        &sidebar_compact,
                        &mut app.settings.sidebar_compact,
                    )
                    .changed()
                    {
                        changed = true;
                    }
                },
            );
            filtered_row(
                ui,
                &palette,
                &needle,
                &appearance,
                &appearance_rows[2],
                |ui| {
                    if widgets::switch(
                        ui,
                        &palette,
                        &tracklist_compact,
                        &mut app.settings.tracklist_compact,
                    )
                    .changed()
                    {
                        changed = true;
                    }
                },
            );
            filtered_row(
                ui,
                &palette,
                &needle,
                &appearance,
                &appearance_rows[3],
                |ui| {
                    ui.horizontal(|ui| {
                        ui.spacing_mut().item_spacing.x = 6.0;
                        let mut zoom = app.settings.zoom;
                        if theme::soft_button(ui, &palette, None, "+", false).clicked() {
                            zoom = (zoom + 0.1).min(2.5);
                        }
                        theme::text(
                            ui,
                            format!("{:.0}%", zoom * 100.0),
                            theme::medium(13.5),
                            palette.text,
                        );
                        if theme::soft_button(ui, &palette, None, "-", false).clicked() {
                            zoom = (zoom - 0.1).max(0.5);
                        }
                        if (zoom - app.settings.zoom).abs() > 0.001 {
                            app.settings.zoom = zoom;
                            ui.ctx().set_zoom_factor(zoom);
                            app.mark_settings_dirty();
                        }
                    });
                },
            );
            if cfg!(target_os = "linux") {
                filtered_row(
                    ui,
                    &palette,
                    &needle,
                    &appearance,
                    &appearance_rows[4],
                    |ui| {
                        if widgets::switch(
                            ui,
                            &palette,
                            &middle_click,
                            &mut app.settings.middle_click_autoscroll,
                        )
                        .changed()
                        {
                            changed = true;
                        }
                    },
                );
            }
            if show_custom_titlebar {
                filtered_row(
                    ui,
                    &palette,
                    &needle,
                    &appearance,
                    &appearance_rows[5],
                    |ui| {
                        let mut custom = app.settings.custom_titlebar;
                        if widgets::switch(ui, &palette, &custom_titlebar, &mut custom).changed() {
                            app.actions.push(Action::SetCustomTitlebar(custom));
                        }
                    },
                );
            }
        });
    }

    let proxy = gettext(locale, "Proxy");
    let proxy_rows = [RowText::new(
        gettext(locale, "Mode, host, port, username and password"),
        gettext(
            locale,
            "Off, System, HTTP or SOCKS5 proxy for network requests.",
        ),
    )];
    if section_matches(&needle, &proxy, &proxy_rows) {
        any_visible = true;
        ui.push_id("proxy-settings", |ui| {
    section(ui, &palette, &proxy, |ui| {
        widgets::setting_row(
            ui,
            &palette,
            &pgettext(locale, "proxy", "Mode"),
            &gettext(
                locale,
                "Off ignores environment variables. System uses them, and the OS proxy on macOS and Windows.",
            ),
            |_| {},
        );
        // The row's control slot is right-to-left and too narrow for four
        // choices; they sit on this line so they read Off, System, HTTP, SOCKS5.
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 6.0;
            for choice in ProxyMode::ALL {
                if theme::soft_button(
                    ui,
                    &palette,
                    None,
                    &choice.label(locale),
                    app.settings.proxy_mode == choice,
                )
                .clicked()
                    && app.settings.proxy_mode != choice
                {
                    app.settings.proxy_mode = choice;
                    changed = true;
                    app.actions.push(Action::ProxyEdited);
                    if choice.is_manual() {
                        proxy_dirty = true;
                    } else {
                        proxy_dirty = false;
                        app.actions.push(Action::ApplyProxy);
                    }
                }
            }
        });
        ui.add_space(10.0);
        if app.settings.proxy_mode.is_manual() {
            if widgets::proxy_manual_form(
                ui,
                &palette,
                app.locale,
                &mut app.settings.proxy_host,
                &mut app.settings.proxy_port,
                &mut app.settings.proxy_username,
                &mut app.settings.proxy_password,
            ) {
                changed = true;
                app.actions.push(Action::ProxyEdited);
                proxy_dirty = true;
            }
            ui.add_space(6.0);
            widgets::proxy_scope_note(ui, &palette, app.locale, app.settings.proxy_mode);
            ui.add_space(10.0);
        }
        if app.settings.proxy_mode.is_manual() {
            ui.horizontal(|ui| {
                if theme::pill_button(ui, &palette, &gettext(locale, "Apply settings"), true).clicked() {
                    app.actions.push(Action::ApplyProxy);
                    if app.settings.proxy_config().is_ok() {
                        proxy_dirty = false;
                    }
                }
            });
        } else {
            theme::subtle(
                ui,
                &palette,
                &match app.settings.proxy_mode {
                    ProxyMode::Off => gettext(locale, "Not using a proxy."),
                    ProxyMode::System => gettext(locale, "Using the system proxy."),
                    ProxyMode::Http | ProxyMode::Socks => "".into(),
                },
            );
        }
    });
        });
    }

    let storage = gettext(locale, "Storage");
    let storage_rows = [
        RowText::new(
            gettext(locale, "Artwork cache"),
            // Translators: {folder} is the path of a cache folder.
            gettext(locale, "Stored in {folder}")
                .replace("{folder}", &app.dirs.art_cache_dir().display().to_string()),
        ),
        RowText::new(
            audio_cache.clone(),
            // Translators: {folder} is the path of a cache folder.
            gettext(locale, "Stored in {folder}").replace(
                "{folder}",
                &app.dirs.audio_cache_dir().display().to_string(),
            ),
        ),
        RowText::new(
            gettext(locale, "Play history"),
            gettext(
                locale,
                // Translators: {file} is the path of the play history file.
                "Tracks played here are stored in {file}. This file is never uploaded.",
            )
            .replace("{file}", &app.dirs.history_file().display().to_string()),
        ),
        RowText::new(
            gettext(locale, "Sign-in"),
            gettext(
                locale,
                "Sign-ins are saved in the system credential store when available.",
            ),
        ),
    ];
    if section_matches(&needle, &storage, &storage_rows) {
        any_visible = true;
        section(ui, &palette, &storage, |ui| {
            filtered_row(ui, &palette, &needle, &storage, &storage_rows[0], |ui| {
                if theme::soft_button(
                    ui,
                    &palette,
                    Some(Icon::Trash),
                    &gettext(locale, "Clear artwork"),
                    false,
                )
                .clicked()
                {
                    app.actions.push(Action::ClearArtCache);
                }
            });
            filtered_row(ui, &palette, &needle, &storage, &storage_rows[1], |_| {});
            filtered_row(ui, &palette, &needle, &storage, &storage_rows[2], |ui| {
                if theme::soft_button(
                    ui,
                    &palette,
                    Some(Icon::Trash),
                    &gettext(locale, "Clear history"),
                    false,
                )
                .clicked()
                {
                    app.actions.push(Action::ClearPlayHistory);
                }
            });
            filtered_row(ui, &palette, &needle, &storage, &storage_rows[3], |_| {});
        });
    }

    let about = gettext(locale, "About");
    let built_with = gettext(
        locale,
        "Built with Rust, egui, and librespot. Not affiliated with Spotify.",
    );
    let check_for_updates = gettext(locale, "Check for updates");
    let checking = gettext(locale, "Checking…");
    let keyboard_shortcuts = gettext(locale, "Keyboard shortcuts");
    let about_rows = [
        RowText::new(
            format!("Spotifast {}", env!("CARGO_PKG_VERSION")),
            built_with.clone(),
        ),
        RowText::new(
            format!("{check_for_updates} {checking}"),
            keyboard_shortcuts.clone(),
        ),
    ];
    if section_matches(&needle, &about, &about_rows) {
        any_visible = true;
        section(ui, &palette, &about, |ui| {
            ui.horizontal(|ui| {
                let (logo, _) = ui.allocate_exact_size(Vec2::splat(40.0), egui::Sense::hover());
                theme::logo(ui, logo.center(), 40.0);
                ui.vertical(|ui| {
                    theme::text(
                        ui,
                        format!("Spotifast {}", env!("CARGO_PKG_VERSION")),
                        theme::semibold(15.0),
                        palette.text,
                    );
                    theme::text(
                        ui,
                        built_with.as_ref(),
                        theme::regular(13.0),
                        palette.secondary,
                    );
                });
            });
            ui.add_space(8.0);
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 8.0;
                let check_label = if app.update_checking {
                    &checking
                } else {
                    &check_for_updates
                };
                if theme::soft_button(ui, &palette, Some(Icon::Refresh), check_label, false)
                    .clicked()
                    && !app.update_checking
                {
                    app.actions.push(Action::CheckForUpdates);
                }
                if theme::soft_button(ui, &palette, Some(Icon::Info), &keyboard_shortcuts, false)
                    .clicked()
                {
                    app.actions.push(Action::ShowDialog(Dialog::Shortcuts));
                }
            });
        });
    }

    if !needle.is_empty() && !any_visible {
        widgets::empty_state(
            ui,
            &palette,
            Icon::Search,
            // Translators: {query} is the text typed into the settings search field.
            &gettext(locale, "No settings for “{query}”").replace("{query}", &needle),
            &gettext(locale, "Try fewer words, or check the spelling."),
        );
    }

    ui.data_mut(|data| data.insert_temp(dirty_id, playback_dirty));
    ui.data_mut(|data| data.insert_temp(proxy_dirty_id, proxy_dirty));
    if changed {
        app.actions.push(Action::SettingsChanged);
    }
}

/// The interface language: System first, then each language by its own name,
/// so a reader can find theirs whatever language the app is showing.
fn language_picker(app: &mut App, ui: &mut egui::Ui) {
    let locale = app.locale;
    let system = pgettext(locale, "language", "System");
    let current = app.settings.language;
    let selected = match current {
        LanguageChoice::System => system.clone(),
        LanguageChoice::Locale(chosen) => chosen.native_name().into(),
    };
    let response = egui::ComboBox::from_id_salt("interface_language")
        .selected_text(selected.as_ref())
        .width(200.0_f32.min(ui.available_width()))
        // As many languages as a menu holds before it scrolls, not five.
        .height(1000.0)
        .show_ui(ui, |ui| {
            let choices = std::iter::once((LanguageChoice::System, system.clone())).chain(
                crate::i18n::LOCALES
                    .iter()
                    .map(|&each| (LanguageChoice::Locale(each), each.native_name().into())),
            );
            for (choice, label) in choices {
                if ui
                    .selectable_label(current == choice, label.as_ref())
                    .clicked()
                    && current != choice
                {
                    app.actions.push(Action::SetLanguage(choice));
                }
            }
        });
    let name = gettext(locale, "Language");
    response.response.widget_info(|| {
        let mut info =
            egui::WidgetInfo::labeled(egui::WidgetType::ComboBox, ui.is_enabled(), name.as_ref());
        info.current_text_value = Some(selected.to_string());
        info
    });
}

#[cfg(test)]
mod tests {
    use super::{RowText, row_matches, section_matches};

    #[test]
    fn empty_filter_matches_everything() {
        assert!(row_matches(
            "",
            "Normalize volume",
            "Keep loud tracks level"
        ));
        assert!(section_matches(
            "",
            "Playback on this computer",
            &[RowText::new("Status", "Spotify")]
        ));
    }

    #[test]
    fn row_matches_title_or_description_case_insensitively() {
        assert!(row_matches(
            "volume",
            "Normalize volume",
            "Keep loud tracks level"
        ));
        assert!(row_matches(
            "VOLUME",
            "Normalize volume",
            "Keep loud tracks level"
        ));
        assert!(row_matches(
            "quiet",
            "Normalize volume",
            "Keep loud and quiet tracks level"
        ));
        assert!(!row_matches(
            "theme",
            "Normalize volume",
            "Keep loud tracks level"
        ));
    }

    #[test]
    fn section_matches_title_or_any_row() {
        let rows = [
            RowText::new("Normalize volume", "Keep loud and quiet tracks level"),
            RowText::new("Autoplay", "Keep playing similar songs"),
        ];
        assert!(section_matches(
            "playback",
            "Playback on this computer",
            &rows
        ));
        assert!(section_matches(
            "autoplay",
            "Playback on this computer",
            &rows
        ));
        assert!(section_matches("quiet", "Playback on this computer", &rows));
        assert!(!section_matches(
            "theme",
            "Playback on this computer",
            &rows
        ));
    }
}
