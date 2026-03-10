mod line;
mod tab;

use std::cmp::{max, min};
use std::collections::BTreeMap;
use std::convert::TryInto;

use tab::get_tab_to_focus;
use zellij_tile::prelude::*;

use crate::line::{tab_line, tab_separator};
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

        // Use two-row mode when the config requests it AND the pane is tall enough.
        // Row 0 (top) = original full tab line: session name + styled tabs (upstream unchanged).
        // Row 1 (bottom) = number hints: tab index numbers with matching arrow separators.
        let effective_rows = self.configured_rows.min(rows);
        if effective_rows >= 2 {
            let row0 = self
                .tab_line
                .iter()
                .fold(String::new(), |output, part| output + &part.part);
            let row1 = build_number_row(&self.tab_line, &self.tabs, &self.mode_info);
            match background {
                PaletteColor::Rgb((r, g, b)) => {
                    print!(
                        "{}\u{1b}[48;2;{};{};{}m\u{1b}[0K\r\n{}\u{1b}[48;2;{};{};{}m\u{1b}[0K",
                        row0, r, g, b, row1, r, g, b
                    );
                },
                PaletteColor::EightBit(color) => {
                    print!(
                        "{}\u{1b}[48;5;{}m\u{1b}[0K\r\n{}\u{1b}[48;5;{}m\u{1b}[0K",
                        row0, color, row1, color
                    );
                },
            }
        } else {
            let output = self
                .tab_line
                .iter()
                .fold(String::new(), |output, part| output + &part.part);
            match background {
                PaletteColor::Rgb((r, g, b)) => {
                    print!("{}\u{1b}[48;2;{};{};{}m\u{1b}[0K", output, r, g, b);
                },
                PaletteColor::EightBit(color) => {
                    print!("{}\u{1b}[48;5;{}m\u{1b}[0K", output, color);
                },
            }
        }
    }
}

/// Builds a number-hints row aligned to the tab segments in `tab_line`.
///
/// For each tab segment: renders `left_arrow + centered_number + right_arrow` using the same
/// powerline arrow characters and background colours as the corresponding tab in row 0.
/// For non-tab segments (session name prefix, fill): renders blank background.
fn build_number_row(tab_line: &[LinePart], tabs: &[TabInfo], mode_info: &ModeInfo) -> String {
    let palette = mode_info.style.colors;
    let fill_bg = palette.text_unselected.background;
    let sep = tab_separator(mode_info.capabilities);
    // Arrow separator is either 1 Unicode char () or empty string.
    let sep_width: usize = if sep.is_empty() { 0 } else { 1 };

    let mut output = String::new();
    for part in tab_line {
        if let Some(tab_idx) = part.tab_index {
            // Determine tab colors the same way render_tab() does in tab.rs:
            // even positions = non-alternate, odd positions = alternate.
            let (bg, fg) = if let Some(tab) = tabs.iter().find(|t| t.position == tab_idx) {
                if tab.active {
                    (palette.ribbon_selected.background, palette.ribbon_selected.base)
                } else if tab_idx % 2 == 1 {
                    (palette.ribbon_unselected.emphasis_1, palette.ribbon_unselected.base)
                } else {
                    (palette.ribbon_unselected.background, palette.ribbon_unselected.base)
                }
            } else {
                (fill_bg, palette.text_unselected.base)
            };

            let inner_width = part.len.saturating_sub(sep_width * 2);
            let num_str = center_in_width(tab_idx + 1, inner_width);

            // Left arrow: bg=tab, fg=fill  (mirrors tab.rs: style!(fill_color, bg).paint(sep))
            output.push_str(&format!(
                "{}{}{}\x1b[0m",
                ansi_color_bg(bg),
                ansi_color_fg(fill_bg),
                sep
            ));
            // Number: bg=tab, fg=tab_text
            output.push_str(&format!(
                "{}{}{}\x1b[0m",
                ansi_color_bg(bg),
                ansi_color_fg(fg),
                num_str
            ));
            // Right arrow: bg=fill, fg=tab  (mirrors tab.rs: style!(bg, fill_color).paint(sep))
            output.push_str(&format!(
                "{}{}{}\x1b[0m",
                ansi_color_bg(fill_bg),
                ansi_color_fg(bg),
                sep
            ));
        } else {
            // Session name prefix, fill, or other non-tab segments → blank background
            let spaces = " ".repeat(part.len);
            output.push_str(&format!("{}{}\x1b[0m", ansi_color_bg(fill_bg), spaces));
        }
    }
    output
}

fn center_in_width(num: usize, width: usize) -> String {
    let s = format!(" {} ", num);
    let slen = s.len(); // ASCII only
    if slen >= width {
        return s.chars().take(width).collect();
    }
    let pad = width - slen;
    let left_pad = pad / 2;
    let right_pad = pad - left_pad;
    format!("{}{}{}", " ".repeat(left_pad), s, " ".repeat(right_pad))
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
