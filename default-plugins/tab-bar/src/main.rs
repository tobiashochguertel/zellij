mod line;
mod tab;

use std::cmp::{max, min};
use std::collections::BTreeMap;
use std::convert::TryInto;

use tab::get_tab_to_focus;
use zellij_tile::prelude::*;

use crate::line::tab_line;
use crate::tab::tab_style;

#[derive(Debug, Default)]
pub struct LinePart {
    part: String,
    len: usize,
    tab_index: Option<usize>,
}

impl LinePart {
    pub fn append(&mut self, to_append: &LinePart) {
        self.part.push_str(&to_append.part);
        self.len += to_append.len;
    }
}

#[derive(Default)]
struct State {
    tabs: Vec<TabInfo>,
    active_tab_idx: usize,
    mode_info: ModeInfo,
    tab_line: Vec<LinePart>,
    hide_swap_layout_indication: bool,
    /// Number of rows to render (1 = original single-row, 2+ = tab index row above tab names row).
    configured_rows: usize,
}

static ARROW_SEPARATOR: &str = "";

register_plugin!(State);

impl ZellijPlugin for State {
    fn load(&mut self, configuration: BTreeMap<String, String>) {
        self.hide_swap_layout_indication = configuration
            .get("hide_swap_layout_indication")
            .map(|s| s == "true")
            .unwrap_or(false);
        self.configured_rows = configuration
            .get("rows")
            .and_then(|s| s.parse::<usize>().ok())
            .unwrap_or(1)
            .max(1);
        request_permission(&[
            PermissionType::ReadApplicationState,
            PermissionType::ChangeApplicationState,
        ]);
        set_selectable(false);
        subscribe(&[
            EventType::TabUpdate,
            EventType::ModeUpdate,
            EventType::Mouse,
        ]);
    }

    fn update(&mut self, event: Event) -> bool {
        let mut should_render = false;
        match event {
            Event::ModeUpdate(mode_info) => {
                if self.mode_info != mode_info {
                    should_render = true;
                }
                self.mode_info = mode_info;
            },
            Event::TabUpdate(tabs) => {
                if let Some(active_tab_index) = tabs.iter().position(|t| t.active) {
                    // tabs are indexed starting from 1 so we need to add 1
                    let active_tab_idx = active_tab_index + 1;

                    if self.active_tab_idx != active_tab_idx || self.tabs != tabs {
                        should_render = true;
                    }
                    self.active_tab_idx = active_tab_idx;
                    self.tabs = tabs;
                } else {
                    eprintln!("Could not find active tab.");
                }
            },
            Event::Mouse(me) => match me {
                Mouse::LeftClick(_, col) => {
                    let tab_to_focus = get_tab_to_focus(&self.tab_line, self.active_tab_idx, col);
                    if let Some(idx) = tab_to_focus {
                        switch_tab_to(idx.try_into().unwrap());
                    }
                },
                Mouse::ScrollUp(_) => {
                    switch_tab_to(min(self.active_tab_idx + 1, self.tabs.len()) as u32);
                },
                Mouse::ScrollDown(_) => {
                    switch_tab_to(max(self.active_tab_idx.saturating_sub(1), 1) as u32);
                },
                _ => {},
            },
            _ => {
                eprintln!("Got unrecognized event: {:?}", event);
            },
        }
        should_render
    }

    fn render(&mut self, rows: usize, cols: usize) {
        if self.tabs.is_empty() {
            return;
        }
        let mut all_tabs: Vec<LinePart> = vec![];
        let mut active_tab_index = 0;
        let mut is_alternate_tab = false;
        for t in &mut self.tabs {
            let mut tabname = t.name.clone();
            if t.active && self.mode_info.mode == InputMode::RenameTab {
                if tabname.is_empty() {
                    tabname = String::from("Enter name...");
                }
                active_tab_index = t.position;
            } else if t.active {
                active_tab_index = t.position;
            }
            let tab = tab_style(
                tabname,
                t,
                is_alternate_tab,
                self.mode_info.style.colors,
                self.mode_info.capabilities,
            );
            is_alternate_tab = !is_alternate_tab;
            all_tabs.push(tab);
        }

        let background = self.mode_info.style.colors.text_unselected.background;

        self.tab_line = tab_line(
            self.mode_info.session_name.as_deref(),
            all_tabs,
            active_tab_index,
            cols.saturating_sub(1),
            self.mode_info.style.colors,
            self.mode_info.capabilities,
            self.mode_info.style.hide_session_name,
            self.tabs.iter().find(|t| t.active),
            &self.mode_info,
            self.hide_swap_layout_indication,
            &background,
        );

        let effective_rows = self.configured_rows.min(rows);

        if effective_rows <= 1 {
            // Original single-row behaviour (unchanged).
            let output = self
                .tab_line
                .iter()
                .fold(String::new(), |out, p| out + &p.part);
            match background {
                PaletteColor::Rgb((r, g, b)) => {
                    print!("{}\u{1b}[48;2;{};{};{}m\u{1b}[0K", output, r, g, b);
                },
                PaletteColor::EightBit(color) => {
                    print!("{}\u{1b}[48;5;{}m\u{1b}[0K", output, color);
                },
            }
            return;
        }

        // Multi-row "tall tab" mode.
        //
        // Every row renders the same powerline arrows at the same column positions, with the same
        // tab colours.  This creates a unified tall-tab visual: each tab is a tall rectangle whose
        // borders span all rows.
        //
        // Row layout:
        //   row 0           → session name prefix (left side) + blank tab bodies (arrows only)
        //   row (rows/2)    → tab names centred inside the tab body (the "name row")
        //   all other rows  → blank prefix + blank tab bodies (arrows only)
        //
        // Mouse clicks work on any row because get_tab_to_focus uses column only.
        let name_row = effective_rows / 2;
        let mut output = String::new();

        for row_idx in 0..effective_rows {
            if row_idx > 0 {
                output.push_str("\r\n");
            }

            let row_str = if row_idx == name_row {
                // Tab names row: tab parts rendered as-is, but the session-name prefix is
                // blanked so it only appears on row 0.
                build_name_row(&self.tab_line, &self.mode_info)
            } else {
                // Body row: arrows at tab boundaries, blank content inside each tab.
                // show_prefix=true only for row 0 so the session name stays top-left.
                build_body_row(
                    &self.tab_line,
                    &self.tabs,
                    &self.mode_info,
                    row_idx == 0,
                )
            };

            output.push_str(&row_str);
            // Clear to end of line with the background colour.
            match background {
                PaletteColor::Rgb((r, g, b)) => {
                    output.push_str(&format!("\u{1b}[48;2;{};{};{}m\u{1b}[0K", r, g, b));
                },
                PaletteColor::EightBit(color) => {
                    output.push_str(&format!("\u{1b}[48;5;{}m\u{1b}[0K", color));
                },
            }
        }

        print!("{}", output);
    }
}

/// Builds the tab-names row: tab `LinePart`s are rendered as-is (with names + arrows), but
/// the session-name prefix (the leading `tab_index = None` parts) is replaced with blank
/// background so the session name appears only on row 0.
fn build_name_row(tab_line: &[LinePart], mode_info: &ModeInfo) -> String {
    let fill_bg = mode_info.style.colors.text_unselected.background;
    let mut output = String::new();
    let mut seen_tab = false;
    for part in tab_line {
        if part.tab_index.is_some() {
            seen_tab = true;
        }
        if part.tab_index.is_none() && !seen_tab {
            // Session name prefix: blank it on this row.
            output.push_str(&format!(
                "{}{}\x1b[0m",
                ansi_color_bg(fill_bg),
                " ".repeat(part.len)
            ));
        } else {
            output.push_str(&part.part);
        }
    }
    output
}

/// Builds a body row: powerline arrows at every tab boundary, blank content inside each tab.
///
/// `show_prefix=true` (row 0 only): session-name/leading `LinePart`s before the first tab are
/// rendered as-is so the session name remains visible on the top row.
/// `show_prefix=false`: those parts become blank background — used for all other body rows.
///
/// The arrow colours match those produced by `tab.rs`, so all rows share identical arrow
/// styling and together form a unified tall-tab visual.
fn build_body_row(
    tab_line: &[LinePart],
    tabs: &[TabInfo],
    mode_info: &ModeInfo,
    show_prefix: bool,
) -> String {
    use crate::line::tab_separator;

    let palette = mode_info.style.colors;
    let fill_bg = palette.text_unselected.background;
    let sep = tab_separator(mode_info.capabilities);
    let sep_width: usize = if sep.is_empty() { 0 } else { 1 };

    let mut output = String::new();
    let mut seen_tab = false;

    for part in tab_line {
        match part.tab_index {
            None => {
                if !seen_tab && show_prefix {
                    // Session name / leading decoration before first tab: keep as-is.
                    output.push_str(&part.part);
                } else {
                    // Trailing fill or suppressed prefix: plain background fill.
                    output.push_str(&format!(
                        "{}{}\x1b[0m",
                        ansi_color_bg(fill_bg),
                        " ".repeat(part.len)
                    ));
                }
            },
            Some(tab_idx) => {
                seen_tab = true;
                let (tab_bg, _fg) = tab_colors(tab_idx, tabs, palette);
                let inner_width = part.len.saturating_sub(sep_width * 2);

                // Left arrow  – fg=fill, bg=tab  → fill→tab colour transition
                output.push_str(&format!(
                    "{}{}{}\x1b[0m",
                    ansi_color_bg(tab_bg),
                    ansi_color_fg(fill_bg),
                    sep
                ));
                // Blank body  – bg=tab
                output.push_str(&format!(
                    "{}{}\x1b[0m",
                    ansi_color_bg(tab_bg),
                    " ".repeat(inner_width)
                ));
                // Right arrow – fg=tab, bg=fill  → tab→fill colour transition
                output.push_str(&format!(
                    "{}{}{}\x1b[0m",
                    ansi_color_bg(fill_bg),
                    ansi_color_fg(tab_bg),
                    sep
                ));
            },
        }
    }
    output
}

/// Returns the (background, foreground) colours for a tab at `tab_idx`.
fn tab_colors(tab_idx: usize, tabs: &[TabInfo], palette: Styling) -> (PaletteColor, PaletteColor) {
    let fill_bg = palette.text_unselected.background;
    if let Some(tab) = tabs.iter().find(|t| t.position == tab_idx) {
        if tab.active {
            (palette.ribbon_selected.background, palette.ribbon_selected.base)
        } else if tab_idx % 2 == 1 {
            (palette.ribbon_unselected.emphasis_1, palette.ribbon_unselected.base)
        } else {
            (palette.ribbon_unselected.background, palette.ribbon_unselected.base)
        }
    } else {
        (fill_bg, palette.text_unselected.base)
    }
}

fn ansi_color_bg(c: PaletteColor) -> String {
    match c {
        PaletteColor::Rgb((r, g, b)) => format!("\x1b[48;2;{};{};{}m", r, g, b),
        PaletteColor::EightBit(n) => format!("\x1b[48;5;{}m", n),
    }
}

fn ansi_color_fg(c: PaletteColor) -> String {
    match c {
        PaletteColor::Rgb((r, g, b)) => format!("\x1b[38;2;{};{};{}m", r, g, b),
        PaletteColor::EightBit(n) => format!("\x1b[38;5;{}m", n),
    }
}
