use std::env;
use std::io::{Read, Write};
use std::os::unix::net::UnixStream;
use std::path::PathBuf;

use anyhow::{Context, Result};
use hyprland::dispatch::{Dispatch as HyprDispatch, DispatchType};
use hyprland::shared::Address;
use log::{debug, info, warn};
use serde::Deserialize;

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

/// Client (window) info deserialized from `j/clients` or `j/activewindow`.
#[derive(Deserialize, Debug, Clone)]
pub struct ClientInfo {
    pub address: Address,
    pub at: (i16, i16),
    pub size: (i16, i16),
    pub workspace: WorkspaceBasic,
    pub title: String,
}

/// Monitor info deserialized from `j/monitors all`.
#[derive(Deserialize, Debug, Clone)]
pub struct MonitorInfo {
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
pub fn get_windows() -> Result<Vec<DesktopWindow>> {
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

    // Filter clients to only those on visible workspaces
    let visible_clients: Vec<&ClientInfo> = client_vec
        .iter()
        .filter(|c| visible_workspaces.contains(&c.workspace.name.as_str()))
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
    fn test_live_hyprland_ipc() {
        if env::var("HYPRLAND_INSTANCE_SIGNATURE").is_err() {
            return;
        }
        let windows = get_windows().expect("get_windows() should succeed on live Hyprland");
        assert!(!windows.is_empty(), "Expected at least one visible window");
    }
}
