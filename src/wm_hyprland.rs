use std::env;
use std::io::{Read, Write};
use std::os::unix::net::UnixStream;
use std::path::PathBuf;

use anyhow::{Context, Result};
use hyprland::dispatch::{Dispatch as HyprDispatch, DispatchType};
use hyprland::shared::Address;
use log::{debug, info, warn};
use serde::Deserialize;

use crate::args::{normalize_address, FilterMode, FilterRule, WindowFilter};
use crate::DesktopWindow;

/// Basic workspace identifier from Hyprland IPC.
///
/// In Hyprland 0.57+, `id` is omitted for `special` and `named` workspaces (and
/// inactive `specialWorkspace` entries on monitors), which breaks the `hyprland`
/// crate's `WorkspaceBasic` deserializer (`missing field 'id'`).
#[derive(Deserialize, Debug, Clone)]
pub struct WorkspaceBasic {
    #[allow(dead_code)]
    pub id: Option<i32>,
    pub name: String,
}

fn deserialize_fullscreen<'de, D>(deserializer: D) -> std::result::Result<u8, D::Error>
where
    D: serde::Deserializer<'de>,
{
    #[derive(Deserialize)]
    #[serde(untagged)]
    enum FullscreenValue {
        Bool(bool),
        Int(u8),
    }
    match Option::<FullscreenValue>::deserialize(deserializer)? {
        Some(FullscreenValue::Bool(b)) => Ok(u8::from(b)),
        Some(FullscreenValue::Int(n)) => Ok(n),
        None => Ok(0),
    }
}

/// Client (window) info deserialized from `j/clients` or `j/activewindow`.
#[derive(Deserialize, Debug, Clone)]
pub struct ClientInfo {
    pub address: Address,
    pub at: (i16, i16),
    pub size: (i16, i16),
    pub workspace: WorkspaceBasic,
    #[serde(default)]
    pub class: String,
    #[serde(default)]
    pub title: String,
    #[serde(default, rename = "initialClass")]
    pub initial_class: String,
    #[serde(default, rename = "initialTitle")]
    pub initial_title: String,
    #[serde(default)]
    pub monitor: Option<i64>,
    #[serde(default)]
    pub pid: Option<i32>,
    #[serde(default)]
    pub tags: Vec<String>,
    #[serde(default)]
    pub floating: bool,
    #[serde(default, deserialize_with = "deserialize_fullscreen")]
    pub fullscreen: u8,
    #[serde(default)]
    pub pinned: bool,
    #[serde(default)]
    pub xwayland: bool,
}

impl ClientInfo {
    /// Check if this client matches a single `WindowFilter`.
    pub fn matches_filter(&self, filter: &WindowFilter, monitors: &[MonitorInfo]) -> bool {
        let matched = match &filter.rule {
            FilterRule::Class(re) => re.is_match(&self.class),
            FilterRule::InitialClass(re) => re.is_match(&self.initial_class),
            FilterRule::Title(re) => re.is_match(&self.title),
            FilterRule::InitialTitle(re) => re.is_match(&self.initial_title),
            FilterRule::Address(addr) => normalize_address(&self.address.to_string()) == *addr,
            FilterRule::WorkspaceId(id) => {
                self.workspace.id == Some(*id) || self.workspace.name == id.to_string()
            }
            FilterRule::WorkspaceName(re) => re.is_match(&self.workspace.name),
            FilterRule::MonitorId(id) => self.monitor == Some(*id),
            FilterRule::MonitorName(re) => self
                .monitor
                .and_then(|mid| monitors.iter().find(|m| m.id == Some(mid)))
                .is_some_and(|m| re.is_match(&m.name)),
            FilterRule::Pid(pid) => self.pid == Some(*pid),
            FilterRule::Tag(re) => self.tags.iter().any(|t| re.is_match(t)),
            FilterRule::Floating(val) => self.floating == *val,
            FilterRule::Pinned(val) => self.pinned == *val,
            FilterRule::XWayland(val) => self.xwayland == *val,
            FilterRule::FullscreenBool(val) => (self.fullscreen != 0) == *val,
            FilterRule::FullscreenMode(mode) => self.fullscreen == *mode,
        };

        if filter.negated {
            !matched
        } else {
            matched
        }
    }

    /// Check if this client matches the provided `WindowFilter`s according to `mode`.
    pub fn matches_filters(
        &self,
        filters: &[WindowFilter],
        mode: FilterMode,
        monitors: &[MonitorInfo],
    ) -> bool {
        if filters.is_empty() {
            return true;
        }
        match mode {
            FilterMode::All => filters.iter().all(|f| self.matches_filter(f, monitors)),
            FilterMode::Any => filters.iter().any(|f| self.matches_filter(f, monitors)),
        }
    }
}

/// Monitor info deserialized from `j/monitors all`.
#[derive(Deserialize, Debug, Clone)]
pub struct MonitorInfo {
    #[serde(default)]
    pub id: Option<i64>,
    pub name: String,
    pub width: u16,
    pub height: u16,
    pub x: i32,
    pub y: i32,
    pub scale: f32,
    pub focused: bool,
    #[serde(rename = "activeWorkspace")]
    pub active_workspace: WorkspaceBasic,
    #[serde(rename = "specialWorkspace")]
    pub special_workspace: WorkspaceBasic,
}

fn hypr_socket_path() -> Result<PathBuf> {
    let mut path = if let Some(runtime_dir) = env::var_os("XDG_RUNTIME_DIR") {
        PathBuf::from(runtime_dir)
    } else if let Ok(uid) = env::var("UID") {
        PathBuf::from(format!("/run/user/{uid}"))
    } else {
        anyhow::bail!("Could not find XDG_RUNTIME_DIR or UID");
    };
    let instance = env::var("HYPRLAND_INSTANCE_SIGNATURE")
        .context("HYPRLAND_INSTANCE_SIGNATURE is not set")?;
    path.push("hypr");
    path.push(instance);
    path.push(".socket.sock");
    Ok(path)
}

fn query_ipc(command: &str) -> Result<String> {
    let socket_path = hypr_socket_path()?;
    let mut stream = UnixStream::connect(&socket_path).with_context(|| {
        format!(
            "Failed to connect to Hyprland socket at {}",
            socket_path.display()
        )
    })?;
    stream.write_all(command.as_bytes())?;
    let mut response = String::new();
    stream.read_to_string(&mut response)?;
    Ok(response)
}

pub fn get_clients() -> Result<Vec<ClientInfo>> {
    let raw = query_ipc("j/clients")?;
    serde_json::from_str(&raw).context("Failed to deserialize Hyprland clients")
}

pub fn get_monitors() -> Result<Vec<MonitorInfo>> {
    let raw = query_ipc("j/monitors all")?;
    serde_json::from_str(&raw).context("Failed to deserialize Hyprland monitors")
}

pub fn get_active_client() -> Result<Option<ClientInfo>> {
    let raw = query_ipc("j/activewindow")?;
    if raw.trim() == "{}" {
        return Ok(None);
    }
    let client: ClientInfo =
        serde_json::from_str(&raw).context("Failed to deserialize active window")?;
    Ok(Some(client))
}

/// Return a list of all visible windows on active workspaces.
#[allow(dead_code)]
pub fn get_windows() -> Result<Vec<DesktopWindow>> {
    get_windows_filtered(&[], FilterMode::All)
}

/// Return a list of visible windows on active workspaces matching `filters` according to `filter_mode`.
pub fn get_windows_filtered(
    filters: &[WindowFilter],
    filter_mode: FilterMode,
) -> Result<Vec<DesktopWindow>> {
    // Get all clients
    let client_vec = get_clients().context("Failed to get clients from Hyprland")?;

    // Get monitors to determine visible workspaces
    let monitor_vec = get_monitors().context("Failed to get monitors from Hyprland")?;

    // Collect active (and any open special) workspace names from all monitors
    let visible_workspaces: Vec<&str> = monitor_vec
        .iter()
        .flat_map(|m| {
            let active = Some(m.active_workspace.name.as_str()).filter(|s| !s.is_empty());
            let special = Some(m.special_workspace.name.as_str()).filter(|s| !s.is_empty());
            active.into_iter().chain(special)
        })
        .collect();

    debug!("Visible workspaces: {:?}", visible_workspaces);

    // Filter clients to only those on visible workspaces and matching user filters
    let visible_clients: Vec<&ClientInfo> = client_vec
        .iter()
        .filter(|c| visible_workspaces.contains(&c.workspace.name.as_str()))
        .filter(|c| c.matches_filters(filters, filter_mode, &monitor_vec))
        .collect();

    debug!("Found {} visible windows", visible_clients.len());

    // Get the currently focused client
    let active_address = get_active_client().ok().flatten().map(|c| c.address);

    // Convert to DesktopWindow
    let mut windows = Vec::new();
    for client in visible_clients {
        // Use the address as a unique ID (convert the string representation to a hash)
        let id = compute_client_id(&client.address);

        let window = DesktopWindow {
            id,
            x_window_id: None, // Wayland doesn't use X11 window IDs
            pos: (client.at.0 as i32, client.at.1 as i32),
            size: (client.size.0 as i32, client.size.1 as i32),
            is_focused: active_address.as_ref() == Some(&client.address),
        };
        debug!("Found window: {:?}", window);
        windows.push(window);
    }

    Ok(windows)
}

/// Compute the ID hash for a client address (must match get_windows logic).
fn compute_client_id(address: &Address) -> i64 {
    use std::collections::hash_map::DefaultHasher;
    use std::hash::{Hash, Hasher};
    let mut hasher = DefaultHasher::new();
    address.to_string().hash(&mut hasher);
    hasher.finish() as i64
}

/// Focus the window with the given address.
///
/// Since Hyprland 0.56 the payload of a `dispatch` IPC request is evaluated as
/// Lua (`return hl.dispatch(<payload>)`), so the legacy string form that
/// `DispatchType::FocusWindow` renders (`focuswindow address:0x…`) is a Lua
/// syntax error. `DispatchType::Custom` passes our own payload through verbatim,
/// which lets us send the Lua dispatcher expression instead.
fn focus(address: &Address) -> Result<()> {
    let dispatcher = format!("hl.dsp.focus({{ window = 'address:{address}' }})");
    debug!("Dispatching `{dispatcher}`");
    HyprDispatch::call(DispatchType::Custom(&dispatcher, ""))?;
    Ok(())
}

/// Focus a specific window by its ID.
pub fn focus_window(window: &DesktopWindow) -> Result<()> {
    debug!(
        "focus_window called with ID: {}, pos: {:?}",
        window.id, window.pos
    );

    let client_vec = get_clients().context("Failed to get clients")?;

    // Debug: print all client IDs
    for c in &client_vec {
        let cid = compute_client_id(&c.address);
        debug!(
            "Client: {} (ws {}), ID: {}, match: {}",
            c.title,
            c.workspace.name,
            cid,
            cid == window.id
        );
    }

    // Find the client that matches this window's ID (hash of address)
    let target_client = client_vec
        .iter()
        .find(|c| compute_client_id(&c.address) == window.id)
        .context("Could not find matching client by ID")?;

    info!(
        "Focusing window: {} at ({}, {})",
        target_client.title, window.pos.0, window.pos.1
    );

    focus(&target_client.address).context("Failed to focus window")?;

    Ok(())
}

/// Swap two windows.
#[allow(dead_code)]
pub fn swap_windows(active_window: &DesktopWindow, window: &DesktopWindow) -> Result<()> {
    let client_vec = get_clients().context("Failed to get clients")?;

    let _active_client = client_vec
        .iter()
        .find(|c| compute_client_id(&c.address) == active_window.id)
        .context("Could not find active client by ID")?;

    let target_client = client_vec
        .iter()
        .find(|c| compute_client_id(&c.address) == window.id)
        .context("Could not find target client by ID")?;

    info!(
        "Swapping windows at ({}, {}) <-> ({}, {})",
        active_window.pos.0, active_window.pos.1, window.pos.0, window.pos.1
    );

    // TODO: Implement proper window swapping for Hyprland.
    // Hyprland 0.56 exposes `hl.dsp.window.swap({ target = … })`, which swaps the
    // *focused* window with the target; wiring that up needs the active window to
    // be focused first. For now, just focus the target window.
    focus(&target_client.address).context("Failed to focus target window")?;

    warn!("Window swapping not fully implemented for Hyprland yet");

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_deserialize_hyprland_057_clients() {
        let json = r#"[{
            "address": "0x555ec23a3bb0",
            "mapped": true,
            "hidden": false,
            "visible": true,
            "acceptsInput": true,
            "at": [3844, 24],
            "size": [2552, 1572],
            "workspace": {
                "address": "1",
                "id": 1,
                "type": "normal",
                "name": "1"
            },
            "floating": false,
            "monitor": 0,
            "class": "neovide",
            "title": "~/.config/hypr/hyprland.lua"
        }, {
            "address": "0x555ec1f73ec0",
            "mapped": true,
            "hidden": false,
            "visible": true,
            "acceptsInput": true,
            "at": [4608, 480],
            "size": [896, 752],
            "workspace": {
                "address": "special:S-signal",
                "type": "special",
                "name": "special:S-signal"
            },
            "floating": true,
            "monitor": 0,
            "class": "signal",
            "title": "Signal (6)"
        }]"#;

        let clients: Vec<ClientInfo> = serde_json::from_str(json).unwrap();
        assert_eq!(clients.len(), 2);
        assert_eq!(clients[0].workspace.id, Some(1));
        assert_eq!(clients[0].workspace.name, "1");
        assert_eq!(clients[1].workspace.id, None);
        assert_eq!(clients[1].workspace.name, "special:S-signal");
    }

    #[test]
    fn test_deserialize_hyprland_057_monitors() {
        let json = r#"[{
            "id": 0,
            "name": "eDP-1",
            "description": "BOE 0x0A1F",
            "width": 2560,
            "height": 1600,
            "refreshRate": 165.00400,
            "x": 3840,
            "y": 0,
            "activeWorkspace": {
                "address": "2",
                "id": 2,
                "type": "normal",
                "name": "2"
            },
            "specialWorkspace": {
                "address": "",
                "type": "",
                "name": ""
            },
            "reserved": [0, 20, 0, 0],
            "scale": 1.0,
            "transform": 0,
            "focused": true
        }]"#;

        let monitors: Vec<MonitorInfo> = serde_json::from_str(json).unwrap();
        assert_eq!(monitors.len(), 1);
        assert_eq!(monitors[0].active_workspace.id, Some(2));
        assert_eq!(monitors[0].active_workspace.name, "2");
        assert_eq!(monitors[0].special_workspace.id, None);
        assert_eq!(monitors[0].special_workspace.name, "");
    }

    #[test]
    fn test_client_filter_matching() {
        use crate::args::parse_window_filter;

        let clients_json = r#"[{
            "address": "0x555ec23a3bb0",
            "at": [3844, 24],
            "size": [2552, 1572],
            "workspace": { "id": 1, "name": "1" },
            "floating": false,
            "fullscreen": 0,
            "pinned": false,
            "xwayland": false,
            "monitor": 0,
            "pid": 4952,
            "class": "neovide",
            "title": "~/.config/hypr/hyprland.lua",
            "initialClass": "neovide",
            "initialTitle": "[Scratch]",
            "tags": ["editor", "dev"]
        }, {
            "address": "0x555ec1f73ec0",
            "at": [4608, 480],
            "size": [896, 752],
            "workspace": { "name": "special:S-signal" },
            "floating": true,
            "fullscreen": 2,
            "pinned": true,
            "xwayland": true,
            "monitor": 1,
            "pid": 1219,
            "class": "signal",
            "title": "Signal (6)",
            "initialClass": "signal-desktop",
            "initialTitle": "Signal",
            "tags": ["chat"]
        }]"#;

        let monitors_json = r#"[{
            "id": 0,
            "name": "eDP-1",
            "width": 2560,
            "height": 1600,
            "x": 0,
            "y": 0,
            "activeWorkspace": { "id": 1, "name": "1" },
            "specialWorkspace": { "name": "" },
            "scale": 1.0,
            "focused": true
        }, {
            "id": 1,
            "name": "DP-1",
            "width": 2560,
            "height": 1440,
            "x": 2560,
            "y": 0,
            "activeWorkspace": { "id": 2, "name": "2" },
            "specialWorkspace": { "name": "special:S-signal" },
            "scale": 1.0,
            "focused": false
        }]"#;

        let clients: Vec<ClientInfo> = serde_json::from_str(clients_json).unwrap();
        let monitors: Vec<MonitorInfo> = serde_json::from_str(monitors_json).unwrap();
        let c0 = &clients[0];
        let c1 = &clients[1];

        // class & negation
        assert!(c0.matches_filter(&parse_window_filter("class:^neo").unwrap(), &monitors));
        assert!(!c1.matches_filter(&parse_window_filter("class:^neo").unwrap(), &monitors));
        assert!(c1.matches_filter(&parse_window_filter("!class:^neo").unwrap(), &monitors));

        // initialClass & initialTitle & title
        assert!(c1.matches_filter(&parse_window_filter("initialClass:desktop$").unwrap(), &monitors));
        assert!(c0.matches_filter(&parse_window_filter("initialTitle:\\[Scratch\\]").unwrap(), &monitors));
        assert!(c0.matches_filter(&parse_window_filter("title:hyprland\\.lua").unwrap(), &monitors));

        // address (with and without 0x)
        assert!(c0.matches_filter(&parse_window_filter("address:0x555ec23a3bb0").unwrap(), &monitors));
        assert!(c0.matches_filter(&parse_window_filter("address:555EC23A3BB0").unwrap(), &monitors));
        assert!(!c1.matches_filter(&parse_window_filter("address:0x555ec23a3bb0").unwrap(), &monitors));

        // workspace (by id and by name regex)
        assert!(c0.matches_filter(&parse_window_filter("workspace:1").unwrap(), &monitors));
        assert!(!c1.matches_filter(&parse_window_filter("workspace:1").unwrap(), &monitors));
        assert!(c1.matches_filter(&parse_window_filter("workspace:^special:").unwrap(), &monitors));

        // monitor (by id and by name regex)
        assert!(c0.matches_filter(&parse_window_filter("monitor:0").unwrap(), &monitors));
        assert!(c0.matches_filter(&parse_window_filter("monitor:^eDP").unwrap(), &monitors));
        assert!(c1.matches_filter(&parse_window_filter("monitor:DP-1").unwrap(), &monitors));

        // pid & tags
        assert!(c0.matches_filter(&parse_window_filter("pid:4952").unwrap(), &monitors));
        assert!(c0.matches_filter(&parse_window_filter("tags:^dev$").unwrap(), &monitors));
        assert!(!c1.matches_filter(&parse_window_filter("tags:^dev$").unwrap(), &monitors));

        // booleans & fullscreen mode
        assert!(c0.matches_filter(&parse_window_filter("floating:false").unwrap(), &monitors));
        assert!(c1.matches_filter(&parse_window_filter("floating:true").unwrap(), &monitors));
        assert!(c1.matches_filter(&parse_window_filter("pinned:true").unwrap(), &monitors));
        assert!(c1.matches_filter(&parse_window_filter("xwayland:true").unwrap(), &monitors));
        assert!(c0.matches_filter(&parse_window_filter("fullscreen:false").unwrap(), &monitors));
        assert!(c1.matches_filter(&parse_window_filter("fullscreen:true").unwrap(), &monitors));
        assert!(c1.matches_filter(&parse_window_filter("fullscreen:2").unwrap(), &monitors));

        // Multiple filters combined with AND (FilterMode::All)
        let filters = vec![
            parse_window_filter("class:neovide").unwrap(),
            parse_window_filter("!floating:true").unwrap(),
            parse_window_filter("tags:editor").unwrap(),
        ];
        assert!(c0.matches_filters(&filters, FilterMode::All, &monitors));
        assert!(!c1.matches_filters(&filters, FilterMode::All, &monitors));

        // Multiple filters combined with OR (FilterMode::Any), e.g., multiple PIDs
        let pid_filters = vec![
            parse_window_filter("pid:4952").unwrap(),
            parse_window_filter("pid:1219").unwrap(),
            parse_window_filter("pid:9999").unwrap(),
        ];
        assert!(c0.matches_filters(&pid_filters, FilterMode::Any, &monitors));
        assert!(c1.matches_filters(&pid_filters, FilterMode::Any, &monitors));
        assert!(!c0.matches_filters(&pid_filters, FilterMode::All, &monitors));

        // Empty filters always match in both modes
        assert!(c0.matches_filters(&[], FilterMode::All, &monitors));
        assert!(c0.matches_filters(&[], FilterMode::Any, &monitors));
    }

    #[test]
    fn test_live_hyprland_ipc() {
        if env::var("HYPRLAND_INSTANCE_SIGNATURE").is_err() {
            return;
        }
        let windows = get_windows().expect("get_windows() should succeed on live Hyprland");
        assert!(!windows.is_empty(), "Expected at least one visible window");
    }
}
