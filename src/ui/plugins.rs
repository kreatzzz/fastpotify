//! The Plugins page: install them, read what they may do, switch and
//! remove them.

use egui::{Align, CornerRadius, Frame, Layout, Margin, Sense, Stroke, Vec2};

use crate::app::App;
use crate::model::Action;
use crate::plugins::manager::Plugin;
use crate::theme::{self, Icon, Palette};

use super::widgets;

const URL_DRAFT_ID: &str = "plugin-url-draft";

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
        .fill(
            palette
                .surface
                .gamma_multiply(if palette.dark { 0.7 } else { 1.0 }),
        )
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
    ui.add_space(8.0);
    theme::text(ui, "Plugins", theme::bold(28.0), palette.text);
    ui.add_space(4.0);
    theme::subtle(
        ui,
        &palette,
        "Plugins run in a sandbox and can only ask Woofer to fetch what they declare.",
    );

    install_controls(app, ui, &palette);

    // Cloned so the rows can act on the app while reading it.
    let plugins = app.plugins.clone();
    section(ui, &palette, "Installed", |ui| {
        if plugins.is_empty() {
            widgets::empty_state(
                ui,
                &palette,
                Icon::Puzzle,
                "No plugins installed",
                "Install one from a URL, or drag a .wasm file onto the window.",
            );
            return;
        }
        for plugin in &plugins {
            plugin_row(app, ui, &palette, plugin);
        }
    });
}

/// The URL field and its pill. The draft lives in egui's memory, so the
/// page holds no state of its own; Enter installs just like the pill.
fn install_controls(app: &mut App, ui: &mut egui::Ui, palette: &Palette) {
    section(ui, palette, "Install", |ui| {
        let draft_id = egui::Id::new(URL_DRAFT_ID);
        let mut url = ui
            .data(|data| data.get_temp::<String>(draft_id))
            .unwrap_or_default();
        let mut requested: Option<String> = None;
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 10.0;
            let response = Frame::new()
                .fill(palette.surface)
                .corner_radius(CornerRadius::same(6))
                .inner_margin(Margin::symmetric(10, 6))
                .show(ui, |ui| {
                    ui.add(
                        egui::TextEdit::singleline(&mut url)
                            .hint_text(
                                egui::RichText::new("https://…/plugin.wasm").color(palette.dim),
                            )
                            .font(theme::regular(13.0))
                            .frame(egui::Frame::NONE)
                            .desired_width((ui.available_width() - 150.0).max(160.0)),
                    )
                })
                .inner;
            let pressed_enter =
                response.lost_focus() && ui.input(|input| input.key_pressed(egui::Key::Enter));
            if pressed_enter || theme::pill_button(ui, palette, "Install from URL", true).clicked()
            {
                let url = url.trim().to_string();
                if !url.is_empty() {
                    requested = Some(url);
                }
            }
        });
        if let Some(url) = requested {
            app.actions.push(Action::InstallPluginUrl(url));
            ui.data_mut(|data| data.insert_temp(draft_id, String::new()));
        } else {
            ui.data_mut(|data| data.insert_temp(draft_id, url));
        }
        ui.add_space(2.0);
        theme::subtle(ui, palette, "…or drag a .wasm file onto the window.");
    });
}

fn plugin_row(app: &mut App, ui: &mut egui::Ui, palette: &Palette, plugin: &Plugin) {
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 8.0;
        ui.vertical(|ui| {
            ui.set_width(ui.available_width() - 200.0);
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 8.0;
                theme::text(ui, plugin.name.clone(), theme::semibold(14.0), palette.text);
                theme::text(
                    ui,
                    format!("v{}", plugin.version),
                    theme::regular(12.5),
                    palette.dim,
                );
            });
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 6.0;
                theme::text(
                    ui,
                    plugin.publisher.clone(),
                    theme::regular(12.5),
                    palette.secondary,
                );
                for capability in &plugin.capabilities {
                    chip(ui, palette, capability_label(capability));
                }
            });
        });
        // The control area lays out right-to-left: add the rightmost item first.
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            ui.spacing_mut().item_spacing.x = 6.0;
            let mut enabled = plugin.enabled;
            if widgets::switch(ui, palette, &mut enabled).changed() {
                app.set_plugin_enabled(&plugin.id, enabled);
            }
            if !plugin.homepage.is_empty()
                && theme::icon_button(
                    ui,
                    Icon::ExternalLink,
                    16.0,
                    palette.secondary,
                    palette.text,
                    "Open homepage",
                )
                .clicked()
            {
                app.actions.push(Action::OpenUrl(plugin.homepage.clone()));
            }
            if plugin.bundled {
                chip(ui, palette, "bundled");
            } else if theme::icon_button(
                ui,
                Icon::Trash,
                16.0,
                palette.secondary,
                palette.text,
                "Remove",
            )
            .clicked()
            {
                app.remove_plugin(&plugin.id);
            }
        });
    });
    ui.add_space(10.0);
}

/// The copy of a capability chip: the prefix names the provider kind, which
/// means nothing to a reader, so it is stripped.
fn capability_label(capability: &str) -> &str {
    capability
        .strip_prefix("translation-provider:")
        .unwrap_or(capability)
}

/// A small quiet pill, for what a plugin may do and for the bundled marker.
fn chip(ui: &mut egui::Ui, palette: &Palette, label: &str) {
    let galley =
        ui.painter()
            .layout_no_wrap(label.to_string(), theme::regular(11.5), palette.secondary);
    let (rect, _) = ui.allocate_exact_size(galley.size() + Vec2::new(12.0, 6.0), Sense::hover());
    ui.painter()
        .rect_filled(rect, rect.height() / 2.0, palette.surface_hover);
    ui.painter().galley(
        rect.center() - galley.size() / 2.0,
        galley,
        palette.secondary,
    );
}
