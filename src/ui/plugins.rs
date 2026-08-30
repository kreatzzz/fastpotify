//! The Plugins page: install them, read what they may do, order who is
//! asked first, and remove them.

use egui::{Align, CornerRadius, Frame, Layout, Margin, Sense, Stroke, Vec2};

use crate::app::App;
use crate::model::Action;
use crate::plugins::manager::Plugin;
use crate::theme::{self, Icon, Palette};

const URL_DRAFT_ID: &str = "plugin-url-draft";

/// The provider kinds, in the order the page shows them: lyrics, then the
/// two per-line aids.
const KINDS: &[(&str, &str)] = &[
    ("lyrics", "Lyrics providers"),
    ("translate", "Translation providers"),
    ("romanize", "Romanization providers"),
];

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
        "Plugins run in a sandbox and can only ask Woofer to fetch what they declare. Each kind asks its providers in order, and the first with an answer wins.",
    );

    install_controls(app, ui, &palette);

    // Cloned so the rows can act on the app while reading it.
    let plugins = app.plugins.clone();
    for (kind, title) in KINDS {
        section(ui, &palette, title, |ui| {
            provider_section(app, ui, &palette, &plugins, kind);
        });
    }
    section(ui, &palette, "Other surfaces", |ui| {
        theme::subtle(ui, &palette, "Sidebar panels — coming soon.");
    });
}

/// One kind's section: its chain, in the order it is asked — the bundled
/// default standing in when the user has ordered none — then any
/// installed provider of the kind outside the chain, with a seat waiting
/// at the back.
fn provider_section(
    app: &mut App,
    ui: &mut egui::Ui,
    palette: &Palette,
    plugins: &[Plugin],
    kind: &str,
) {
    let resolved = crate::plugins::manager::chain_plugins(
        plugins,
        &crate::plugins::manager::chain_ids(&app.settings.provider_chains, kind),
        kind,
    );
    if resolved.is_empty() {
        theme::subtle(ui, palette, "No plugins — the built-in source answers.");
    }
    let last = resolved.len().saturating_sub(1);
    for (index, plugin) in resolved.iter().enumerate() {
        plugin_row(
            app,
            ui,
            palette,
            plugin,
            kind,
            RowControls {
                can_move_up: index > 0,
                can_move_down: index < last,
                outside_chain: false,
            },
        );
    }
    let wanted = crate::plugins::PluginManifest::provider_capability(kind);
    for plugin in plugins {
        if !resolved.iter().any(|held| held.id == plugin.id)
            && plugin.capabilities.contains(&wanted)
        {
            plugin_row(
                app,
                ui,
                palette,
                plugin,
                kind,
                RowControls {
                    can_move_up: false,
                    can_move_down: false,
                    outside_chain: true,
                },
            );
        }
    }
    ui.add_space(2.0);
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
        ui.add_space(6.0);
        ui.horizontal(|ui| {
            theme::subtle(ui, palette, "The catalog lives at");
            if theme::text(ui, "usewoofer.com", theme::medium(13.0), palette.accent)
                .interact(egui::Sense::click())
                .clicked()
            {
                app.actions
                    .push(Action::OpenUrl("https://usewoofer.com".into()));
            }
            theme::subtle(ui, palette, "— every listing is reviewed by hand.");
        });
    });
}

/// How one row's controls read: seated in the chain it moves up or down;
/// outside it, there is only the seat at the back to take.
struct RowControls {
    can_move_up: bool,
    can_move_down: bool,
    outside_chain: bool,
}

/// One provider's row. The row acts on the kind of the section it sits
/// in, not the plugin's first claim.
fn plugin_row(
    app: &mut App,
    ui: &mut egui::Ui,
    palette: &Palette,
    plugin: &Plugin,
    kind: &str,
    controls: RowControls,
) {
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 8.0;
        ui.vertical(|ui| {
            ui.set_width(ui.available_width() - 240.0);
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
            if controls.outside_chain {
                if theme::icon_button(
                    ui,
                    Icon::Plus,
                    16.0,
                    palette.secondary,
                    palette.text,
                    "Ask this one last",
                )
                .clicked()
                {
                    add_to_chain(app, kind, &plugin.id);
                }
            } else {
                if !plugin.bundled
                    && theme::icon_button(
                        ui,
                        Icon::Trash,
                        16.0,
                        palette.secondary,
                        palette.text,
                        "Remove",
                    )
                    .clicked()
                {
                    uninstall(app, &plugin.id);
                }
                if controls.can_move_down
                    && theme::icon_button(
                        ui,
                        Icon::ChevronDown,
                        16.0,
                        palette.secondary,
                        palette.text,
                        "Ask this one later",
                    )
                    .clicked()
                {
                    move_in_chain(app, kind, &plugin.id, false);
                }
                if controls.can_move_up
                    && theme::icon_button(
                        ui,
                        Icon::ChevronUp,
                        16.0,
                        palette.secondary,
                        palette.text,
                        "Ask this one first",
                    )
                    .clicked()
                {
                    move_in_chain(app, kind, &plugin.id, true);
                }
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
            }
        });
    });
    ui.add_space(10.0);
}

/// Swaps a provider with its neighbour in its kind's chain. The order
/// lives in the settings, so the ordinary persistence carries it to disk.
fn move_in_chain(app: &mut App, kind: &str, id: &str, up: bool) {
    let chain = app.settings.provider_chains.for_kind_mut(kind);
    let Some(index) = chain.iter().position(|held| held == id) else {
        return;
    };
    let target = if up {
        index.checked_sub(1)
    } else {
        Some(index + 1).filter(|target| *target < chain.len())
    };
    if let Some(target) = target {
        chain.swap(index, target);
        app.mark_settings_dirty();
    }
}

/// Gives a provider the back of its kind's chain: it answers after
/// everyone the user has already ordered.
fn add_to_chain(app: &mut App, kind: &str, id: &str) {
    let chain = app.settings.provider_chains.for_kind_mut(kind);
    if !chain.iter().any(|held| held == id) {
        chain.push(id.to_string());
        app.mark_settings_dirty();
    }
}

/// An uninstall is the file and its seat: the plugin leaves every chain
/// it sat in before its files go.
fn uninstall(app: &mut App, id: &str) {
    app.settings.provider_chains.drop_id(id);
    app.mark_settings_dirty();
    app.remove_plugin(id);
}

/// The copy of a capability chip: the prefix names the provider kind, which
/// means nothing to a reader, so it is stripped.
fn capability_label(capability: &str) -> &str {
    capability
        .strip_prefix(crate::plugins::PROVIDER_CAPABILITY)
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
