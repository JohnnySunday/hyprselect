use anyhow::{Context, Result};
use clap::{Parser, ValueEnum};
use css_color_parser::Color as CssColor;
use regex::Regex;

#[cfg(feature = "i3")]
use font_loader::system_fonts;

#[cfg(feature = "i3")]
use log::{info, warn};

use crate::utils;

#[derive(ValueEnum, Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum HorizontalAlign {
    Left,
    Center,
    Right,
}

#[derive(ValueEnum, Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum VerticalAlign {
    Top,
    Center,
    Bottom,
}

#[derive(ValueEnum, Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum FilterMode {
    /// Window must match all filters (AND)
    #[default]
    #[value(alias = "and")]
    All,
    /// Window must match at least one filter (OR)
    #[value(alias = "or")]
    Any,
}

/// Load a system font.
#[cfg(feature = "i3")]
fn load_font(font_family: &str) -> Result<Vec<u8>> {
    let mut font_family_property = system_fonts::FontPropertyBuilder::new()
        .family(font_family)
        .build();
    let info = system_fonts::query_specific(&mut font_family_property);
    info!("Returned effective font is: {:?}", info);
    let (loaded_font, _) =
        if let Some((loaded_font, index)) = system_fonts::get(&font_family_property) {
            (loaded_font, index)
        } else {
            warn!("Family not found, falling back to first Monospace font");
            let mut font_monospace_property =
                system_fonts::FontPropertyBuilder::new().monospace().build();
            let sysfonts = system_fonts::query_specific(&mut font_monospace_property);
            warn!("Falling back to font '{font}'", font = sysfonts[0]);
            let (loaded_font, index) = system_fonts::get(&font_monospace_property)
                .context("Couldn't find suitable font")?;
            (loaded_font, index)
        };
    Ok(loaded_font)
}

/// Generate a valid `FontConfig` from `f`.
/// `f` is expected to be in format `Mono:72`.
fn parse_truetype_font(f: &str) -> Result<FontConfig> {
    let mut v = f.split(':');
    let (family, size) = (
        v.next().context("Wrong font format")?,
        v.next().context("Wrong font format")?,
    );
    let font_size = size.parse::<f64>().context("Couldn't parse font size")?;

    #[cfg(feature = "hyprland")]
    let loaded_font: Vec<u8> = Vec::new();

    #[cfg(feature = "i3")]
    let loaded_font = load_font(family).context("Couldn't load font")?;

    #[cfg(not(any(feature = "i3", feature = "hyprland")))]
    let loaded_font: Vec<u8> = Vec::new();

    Ok(FontConfig {
        font_family: family.to_string(),
        font_size,
        loaded_font,
    })
}

/// Validate coordinates and parse offset.
fn parse_offset(c: &str) -> Result<Offset, String> {
    let mut v = c.split(',');
    let (x, y) = (
        v.next()
            .ok_or("Wrong coordinate format, expected x,y coordinates")?,
        v.next()
            .ok_or("Wrong coordinate format, expected x,y coordinates")?,
    );
    let offset = Offset {
        x: x.parse::<i32>()
            .map_err(|_| "Couldn't parse x coordinate")?,
        y: y.parse::<i32>()
            .map_err(|_| "Couldn't parse y coordinate")?,
    };
    Ok(offset)
}

/// Parse a color into a tuple of floats.
fn parse_color(color_str: &str) -> Result<(f64, f64, f64, f64), String> {
    let color = color_str
        .parse::<CssColor>()
        .map_err(|_| "Invalid color format")?;
    Ok((
        f64::from(color.r) / 255.0,
        f64::from(color.g) / 255.0,
        f64::from(color.b) / 255.0,
        f64::from(color.a),
    ))
}

#[derive(Debug, Clone)]
pub struct Offset {
    #[allow(dead_code)]
    pub x: i32,
    #[allow(dead_code)]
    pub y: i32,
}

#[derive(Debug, Clone)]
#[allow(dead_code)]
pub struct Margin {
    pub left: f32,
    pub right: f32,
    pub top: f32,
    pub bottom: f32,
}

/// Parse margin in format "left,right,top,bottom" or single value for all sides.
fn parse_margin(s: &str) -> Result<Margin, String> {
    let parts: Vec<&str> = s.split(',').collect();
    match parts.len() {
        1 => {
            let val = parts[0]
                .parse::<f32>()
                .map_err(|_| "Couldn't parse margin value")?;
            Ok(Margin {
                left: val,
                right: val,
                top: val,
                bottom: val,
            })
        }
        4 => {
            let left = parts[0]
                .parse::<f32>()
                .map_err(|_| "Couldn't parse left margin")?;
            let right = parts[1]
                .parse::<f32>()
                .map_err(|_| "Couldn't parse right margin")?;
            let top = parts[2]
                .parse::<f32>()
                .map_err(|_| "Couldn't parse top margin")?;
            let bottom = parts[3]
                .parse::<f32>()
                .map_err(|_| "Couldn't parse bottom margin")?;
            Ok(Margin {
                left,
                right,
                top,
                bottom,
            })
        }
        _ => Err("Wrong margin format, expected single value or left,right,top,bottom".to_string()),
    }
}

#[derive(Debug, Clone)]
pub struct FontConfig {
    #[allow(dead_code)]
    pub font_family: String,
    #[allow(dead_code)]
    pub font_size: f64,
    #[allow(dead_code)]
    pub loaded_font: Vec<u8>,
}

fn parse_exit_keys(s: &str) -> Result<utils::Sequence> {
    Ok(utils::Sequence::new(Some(s)))
}

#[derive(Debug, Clone)]
#[allow(dead_code)]
pub enum FilterRule {
    Class(Regex),
    InitialClass(Regex),
    Title(Regex),
    InitialTitle(Regex),
    Address(String),
    WorkspaceId(i32),
    WorkspaceName(Regex),
    MonitorId(i64),
    MonitorName(Regex),
    Pid(i32),
    Tag(Regex),
    Floating(bool),
    Pinned(bool),
    XWayland(bool),
    FullscreenBool(bool),
    FullscreenMode(u8),
}

#[derive(Debug, Clone)]
#[allow(dead_code)]
pub struct WindowFilter {
    pub negated: bool,
    pub rule: FilterRule,
}

fn compile_regex(pattern: &str) -> Result<Regex, String> {
    Regex::new(pattern).map_err(|e| format!("Invalid regex '{pattern}': {e}"))
}

fn parse_bool_value(s: &str) -> Result<bool, String> {
    match s.trim().to_ascii_lowercase().as_str() {
        "true" | "1" | "yes" | "on" => Ok(true),
        "false" | "0" | "no" | "off" => Ok(false),
        _ => Err(format!(
            "Invalid boolean value '{s}', expected true/false, 1/0, yes/no, or on/off"
        )),
    }
}

pub fn normalize_address(addr: &str) -> String {
    addr.trim()
        .trim_start_matches("0x")
        .trim_start_matches("0X")
        .to_ascii_lowercase()
}

/// Parse a window filter in format `[!]<property>:<value>`.
pub fn parse_window_filter(s: &str) -> Result<WindowFilter, String> {
    let (negated, rest) = if let Some(stripped) = s.strip_prefix('!') {
        (true, stripped)
    } else {
        (false, s)
    };

    let (prop_raw, val) = rest
        .split_once(':')
        .ok_or_else(|| format!("Invalid filter '{s}': expected '[!]<property>:<value>'"))?;

    let prop_norm: String = prop_raw
        .trim()
        .chars()
        .filter(|c| *c != '_' && *c != '-')
        .map(|c| c.to_ascii_lowercase())
        .collect();

    let rule = match prop_norm.as_str() {
        "class" => FilterRule::Class(compile_regex(val)?),
        "initialclass" => FilterRule::InitialClass(compile_regex(val)?),
        "title" => FilterRule::Title(compile_regex(val)?),
        "initialtitle" => FilterRule::InitialTitle(compile_regex(val)?),
        "address" | "addr" => {
            let norm = normalize_address(val);
            if norm.is_empty() {
                return Err("Address filter value cannot be empty".to_string());
            }
            FilterRule::Address(norm)
        }
        "workspace" | "ws" => {
            if let Ok(id) = val.trim().parse::<i32>() {
                FilterRule::WorkspaceId(id)
            } else {
                FilterRule::WorkspaceName(compile_regex(val)?)
            }
        }
        "monitor" | "mon" => {
            if let Ok(id) = val.trim().parse::<i64>() {
                FilterRule::MonitorId(id)
            } else {
                FilterRule::MonitorName(compile_regex(val)?)
            }
        }
        "pid" => {
            let pid = val
                .trim()
                .parse::<i32>()
                .map_err(|_| format!("Invalid pid '{val}': expected integer"))?;
            FilterRule::Pid(pid)
        }
        "tag" | "tags" => FilterRule::Tag(compile_regex(val)?),
        "floating" => FilterRule::Floating(parse_bool_value(val)?),
        "pinned" => FilterRule::Pinned(parse_bool_value(val)?),
        "xwayland" => FilterRule::XWayland(parse_bool_value(val)?),
        "fullscreen" => {
            if let Ok(mode) = val.trim().parse::<u8>() {
                FilterRule::FullscreenMode(mode)
            } else {
                FilterRule::FullscreenBool(parse_bool_value(val)?)
            }
        }
        _ => {
            return Err(format!(
                "Unknown filter property '{prop_raw}'. Supported properties: class, initialClass, title, initialTitle, address, workspace, monitor, pid, tags, floating, fullscreen, pinned, xwayland"
            ))
        }
    };

    Ok(WindowFilter { negated, rule })
}

#[derive(Parser, Debug)]
#[command(name = "hyprselect", author, about, version)]
pub struct AppConfig {
    /// Use a specific TrueType font with this format: family:size
    #[arg(
        short,
        long,
        default_value = "Mono:72",
        value_parser(parse_truetype_font)
    )]
    pub font: FontConfig,

    /// Define a set of possbile values to use as hint characters
    #[arg(short = 'c', long = "chars", default_value = "sadfjklewcmpgh")]
    pub hint_chars: String,

    /// Add an additional margin around the text box (value is a factor of the box size)
    /// Format: single value or left,right,top,bottom
    #[arg(short, long, default_value = "0.2", value_parser(parse_margin))]
    pub margin: Margin,

    /// Text color (CSS notation)
    #[arg(
        long = "textcolor",
        display_order = 49,
        default_value = "#dddddd",
        value_parser(parse_color)
    )]
    pub text_color: (f64, f64, f64, f64),

    /// Text color alternate (CSS notation)
    #[arg(
        long = "textcoloralt",
        display_order = 50,
        default_value = "#666666",
        value_parser(parse_color)
    )]
    pub text_color_alt: (f64, f64, f64, f64),

    /// Background color (CSS notation)
    #[arg(
        long = "bgcolor",
        display_order = 51,
        default_value = "rgba(30, 30, 30, 0.9)",
        value_parser(parse_color)
    )]
    pub bg_color: (f64, f64, f64, f64),

    /// Text color current window (CSS notation)
    #[arg(
        long = "textcolorcurrent",
        display_order = 52,
        default_value = "#333333",
        value_parser(parse_color)
    )]
    pub text_color_current: (f64, f64, f64, f64),

    /// Text color current window alternate (CSS notation)
    #[arg(
        long = "textcolorcurrentalt",
        display_order = 53,
        default_value = "#999999",
        value_parser(parse_color)
    )]
    pub text_color_current_alt: (f64, f64, f64, f64),

    /// Background color current window (CSS notation)
    #[arg(
        long = "bgcolorcurrent",
        display_order = 54,
        default_value = "rgba(200, 200, 200, 0.9)",
        value_parser(parse_color)
    )]
    pub bg_color_current: (f64, f64, f64, f64),

    /// Horizontal alignment of the box inside the window
    #[arg(
        long = "halign",
        display_order = 100,
        default_value = "left",
        ignore_case = true
    )]
    pub horizontal_align: HorizontalAlign,

    /// Vertical alignment of the box inside the window
    #[arg(
        long = "valign",
        display_order = 101,
        default_value = "top",
        ignore_case = true
    )]
    pub vertical_align: VerticalAlign,

    /// Completely fill out windows
    #[arg(long, display_order = 102, conflicts_with_all(&["horizontal_align", "vertical_align", "margin", "offset"]))]
    pub fill: bool,

    /// Print the window id only but don't change focus
    #[arg(short, long)]
    pub print_only: bool,

    /// Offset box from edge of window relative to alignment (x,y)
    #[arg(
        short,
        long,
        allow_hyphen_values = true,
        default_value = "0,0",
        value_parser(parse_offset)
    )]
    pub offset: Offset,

    /// List of keys to exit application, sequences separator is space, key separator is '+', eg Control_L+g Shift_L+f
    #[arg(short, long, value_parser(parse_exit_keys))]
    pub exit_keys: Vec<utils::Sequence>,

    /// If this flag is set, the currently active window will swap with the selected window.
    #[arg(short, long)]
    pub swap: bool,

    /// Filter Hyprland windows by property (format: [!]<property>:<value>, repeatable).
    /// Supported properties: class, initialClass, title, initialTitle, address, workspace, monitor, pid, tags, floating, fullscreen, pinned, xwayland
    #[arg(
        short = 'F',
        long = "filter",
        value_name = "[!]<PROP>:<VALUE>",
        value_parser(parse_window_filter)
    )]
    pub filters: Vec<WindowFilter>,

    /// How multiple --filter flags are combined ('all'/'and' or 'any'/'or')
    #[arg(
        long = "filter-mode",
        default_value = "all",
        ignore_case = true
    )]
    pub filter_mode: FilterMode,

    /// Match windows that satisfy any --filter flag (shorthand for --filter-mode any)
    #[arg(long = "filter-any", conflicts_with = "filter_mode")]
    pub filter_any: bool,
}

#[cfg(any(feature = "i3", feature = "hyprland"))]
pub fn parse_args() -> AppConfig {
    let mut config = AppConfig::parse();
    if config.fill {
        config.horizontal_align = HorizontalAlign::Center;
        config.vertical_align = VerticalAlign::Center;
    }
    if config.filter_any {
        config.filter_mode = FilterMode::Any;
    }
    config
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_window_filter_regex_and_negation() {
        let f = parse_window_filter("class:^firefox$").unwrap();
        assert!(!f.negated);
        assert!(matches!(f.rule, FilterRule::Class(_)));

        let f_neg = parse_window_filter("!title:.*vim.*").unwrap();
        assert!(f_neg.negated);
        assert!(matches!(f_neg.rule, FilterRule::Title(_)));

        let f_ic = parse_window_filter("initialClass:kitty").unwrap();
        assert!(!f_ic.negated);
        assert!(matches!(f_ic.rule, FilterRule::InitialClass(_)));

        let f_it = parse_window_filter("initial_title:Scratch").unwrap();
        assert!(!f_it.negated);
        assert!(matches!(f_it.rule, FilterRule::InitialTitle(_)));
    }

    #[test]
    fn test_parse_window_filter_exact_and_bool() {
        let f_addr = parse_window_filter("address:0x555ec23a3bb0").unwrap();
        assert!(matches!(f_addr.rule, FilterRule::Address(ref a) if a == "555ec23a3bb0"));

        let f_ws_id = parse_window_filter("workspace:2").unwrap();
        assert!(matches!(f_ws_id.rule, FilterRule::WorkspaceId(2)));

        let f_ws_name = parse_window_filter("workspace:special:.*").unwrap();
        assert!(matches!(f_ws_name.rule, FilterRule::WorkspaceName(_)));

        let f_mon_id = parse_window_filter("monitor:0").unwrap();
        assert!(matches!(f_mon_id.rule, FilterRule::MonitorId(0)));

        let f_mon_name = parse_window_filter("monitor:eDP-1").unwrap();
        assert!(matches!(f_mon_name.rule, FilterRule::MonitorName(_)));

        let f_pid = parse_window_filter("pid:4952").unwrap();
        assert!(matches!(f_pid.rule, FilterRule::Pid(4952)));

        let f_tag = parse_window_filter("tags:browser").unwrap();
        assert!(matches!(f_tag.rule, FilterRule::Tag(_)));

        let f_float = parse_window_filter("floating:true").unwrap();
        assert!(matches!(f_float.rule, FilterRule::Floating(true)));

        let f_fs_bool = parse_window_filter("fullscreen:false").unwrap();
        assert!(matches!(f_fs_bool.rule, FilterRule::FullscreenBool(false)));

        let f_fs_mode = parse_window_filter("fullscreen:2").unwrap();
        assert!(matches!(f_fs_mode.rule, FilterRule::FullscreenMode(2)));
    }

    #[test]
    fn test_parse_window_filter_errors() {
        assert!(parse_window_filter("invalid_no_colon").is_err());
        assert!(parse_window_filter("unknown_prop:val").is_err());
        assert!(parse_window_filter("class:[unclosed").is_err());
        assert!(parse_window_filter("pid:not_an_int").is_err());
        assert!(parse_window_filter("floating:maybe").is_err());
        assert!(parse_window_filter("address:0x").is_err());
    }
}
