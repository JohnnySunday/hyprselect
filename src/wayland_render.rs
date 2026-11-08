use anyhow::{Context, Result};
use log::info;
use std::collections::HashMap;
use std::os::fd::AsFd;
use std::sync::{Arc, Mutex};

use wayland_client::{
    globals::{registry_queue_init, GlobalListContents},
    protocol::{
        wl_buffer, wl_compositor, wl_keyboard, wl_output, wl_registry, wl_seat, wl_shm,
        wl_shm_pool, wl_surface,
    },
    Connection, Dispatch, QueueHandle,
};

use wayland_protocols_wlr::layer_shell::v1::client::{
    zwlr_layer_shell_v1::{self, Layer},
    zwlr_layer_surface_v1::{self, Anchor, KeyboardInteractivity},
};

use crate::{args::AppConfig, DesktopWindow};

pub struct WaylandRenderer {
    app_config: AppConfig,
}

/// Data collected from wl_output events (Geometry position + Name for monitor matching).
#[derive(Default)]
struct OutputData {
    x: i32,
    y: i32,
    name: String,
}

/// Logical size and offset of a per-monitor layer surface, populated after Configure.
struct SurfaceConfig {
    width: i32,
    height: i32,
    offset_x: i32,
    offset_y: i32,
}

struct RenderState {
    _compositor: wl_compositor::WlCompositor,
    _shm: wl_shm::WlShm,
    _layer_shell: zwlr_layer_shell_v1::ZwlrLayerShellV1,
    _seat: Option<wl_seat::WlSeat>,
    /// One entry per layer surface; width/height filled in on Configure.
    surfaces: Vec<SurfaceConfig>,
    configured_count: usize,
    total_surfaces: usize,
    keyboard_state: Option<KeyboardState>,
    pressed_keys: String,
    should_exit: bool,
}

struct KeyboardState {
    xkb_context: xkbcommon::xkb::Context,
    xkb_state: Option<xkbcommon::xkb::State>,
}

// SAFETY: xkbcommon is internally thread-safe; the raw pointers it wraps are
// protected by the library. We operate single-threaded, so this is sound.
unsafe impl Send for RenderState {}
unsafe impl Sync for RenderState {}

// ── Dispatch implementations ─────────────────────────────────────────────────

impl Dispatch<wl_registry::WlRegistry, GlobalListContents> for RenderState {
    fn event(
        _: &mut Self,
        _: &wl_registry::WlRegistry,
        _: wl_registry::Event,
        _: &GlobalListContents,
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
    }
}

impl Dispatch<wl_compositor::WlCompositor, ()> for RenderState {
    fn event(
        _: &mut Self,
        _: &wl_compositor::WlCompositor,
        _: wl_compositor::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
    }
}

impl Dispatch<wl_surface::WlSurface, ()> for RenderState {
    fn event(
        _: &mut Self,
        _: &wl_surface::WlSurface,
        _: wl_surface::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
    }
}

impl Dispatch<wl_shm::WlShm, ()> for RenderState {
    fn event(
        _: &mut Self,
        _: &wl_shm::WlShm,
        _: wl_shm::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
    }
}

impl Dispatch<wl_shm_pool::WlShmPool, ()> for RenderState {
    fn event(
        _: &mut Self,
        _: &wl_shm_pool::WlShmPool,
        _: wl_shm_pool::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
    }
}

impl Dispatch<wl_buffer::WlBuffer, ()> for RenderState {
    fn event(
        _: &mut Self,
        _: &wl_buffer::WlBuffer,
        _: wl_buffer::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
    }
}

impl Dispatch<zwlr_layer_shell_v1::ZwlrLayerShellV1, ()> for RenderState {
    fn event(
        _: &mut Self,
        _: &zwlr_layer_shell_v1::ZwlrLayerShellV1,
        _: zwlr_layer_shell_v1::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
    }
}

/// The userdata is the index into `state.surfaces` so we know which surface was configured.
impl Dispatch<zwlr_layer_surface_v1::ZwlrLayerSurfaceV1, usize> for RenderState {
    fn event(
        state: &mut Self,
        layer_surface: &zwlr_layer_surface_v1::ZwlrLayerSurfaceV1,
        event: zwlr_layer_surface_v1::Event,
        idx: &usize,
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        if let zwlr_layer_surface_v1::Event::Configure {
            serial,
            width,
            height,
        } = event
        {
            layer_surface.ack_configure(serial);
            if *idx < state.surfaces.len() {
                state.surfaces[*idx].width = width as i32;
                state.surfaces[*idx].height = height as i32;
            }
            state.configured_count += 1;
        }
    }
}

impl Dispatch<wl_seat::WlSeat, ()> for RenderState {
    fn event(
        _: &mut Self,
        _: &wl_seat::WlSeat,
        _: wl_seat::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
    }
}

/// Capture the output's global logical position (Geometry) and name (version 4+).
impl Dispatch<wl_output::WlOutput, Arc<Mutex<OutputData>>> for RenderState {
    fn event(
        _: &mut Self,
        _: &wl_output::WlOutput,
        event: wl_output::Event,
        data: &Arc<Mutex<OutputData>>,
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        let mut d = data.lock().unwrap();
        match event {
            wl_output::Event::Geometry { x, y, .. } => {
                d.x = x;
                d.y = y;
            }
            wl_output::Event::Name { name } => {
                d.name = name;
            }
            _ => {}
        }
    }
}

impl Dispatch<wl_keyboard::WlKeyboard, ()> for RenderState {
    fn event(
        state: &mut Self,
        _keyboard: &wl_keyboard::WlKeyboard,
        event: wl_keyboard::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        use xkbcommon::xkb;

        match event {
            wl_keyboard::Event::Keymap {
                format: wayland_client::WEnum::Value(wl_keyboard::KeymapFormat::XkbV1),
                fd,
                size,
            } => {
                let keymap_data = unsafe {
                    let ptr = nix::sys::mman::mmap(
                        None,
                        std::num::NonZeroUsize::new(size as usize).unwrap(),
                        nix::sys::mman::ProtFlags::PROT_READ,
                        nix::sys::mman::MapFlags::MAP_PRIVATE,
                        fd.as_fd(),
                        0,
                    )
                    .expect("mmap failed");

                    let slice =
                        std::slice::from_raw_parts(ptr.as_ptr() as *const u8, size as usize - 1);
                    let keymap_str = std::str::from_utf8_unchecked(slice);
                    let result = keymap_str.to_string();

                    nix::sys::mman::munmap(ptr, size as usize).expect("munmap failed");
                    result
                };

                if let Some(kb_state) = &mut state.keyboard_state {
                    let keymap = xkb::Keymap::new_from_string(
                        &kb_state.xkb_context,
                        keymap_data,
                        xkb::KEYMAP_FORMAT_TEXT_V1,
                        xkb::KEYMAP_COMPILE_NO_FLAGS,
                    )
                    .expect("Failed to create keymap");

                    kb_state.xkb_state = Some(xkb::State::new(&keymap));
                }
            }

            wl_keyboard::Event::Key {
                key,
                state: key_state,
                ..
            } => {
                if let Some(kb_state) = &mut state.keyboard_state {
                    if let Some(xkb_state) = &mut kb_state.xkb_state {
                        let keycode = key + 8;

                        if let wayland_client::WEnum::Value(wl_keyboard::KeyState::Pressed) =
                            key_state
                        {
                            let keysym = xkb_state.key_get_one_sym(xkb::Keycode::from(keycode));
                            let keysym_name = xkb::keysym_get_name(keysym);

                            if keysym == xkb::keysyms::KEY_Escape.into() {
                                state.should_exit = true;
                                return;
                            }

                            if keysym == xkb::keysyms::KEY_BackSpace.into() {
                                state.pressed_keys.pop();
                                return;
                            }

                            if keysym_name.len() == 1 {
                                state.pressed_keys.push_str(&keysym_name.to_lowercase());
                            }
                        }
                    }
                }
            }

            wl_keyboard::Event::Modifiers {
                mods_depressed,
                mods_latched,
                mods_locked,
                group,
                ..
            } => {
                if let Some(kb_state) = &mut state.keyboard_state {
                    if let Some(xkb_state) = &mut kb_state.xkb_state {
                        xkb_state.update_mask(
                            mods_depressed,
                            mods_latched,
                            mods_locked,
                            0,
                            0,
                            group,
                        );
                    }
                }
            }

            _ => {}
        }
    }
}

// ── WaylandRenderer ──────────────────────────────────────────────────────────

impl WaylandRenderer {
    pub fn new(app_config: AppConfig) -> Result<Self> {
        Ok(Self { app_config })
    }

    pub fn render_hints(
        &mut self,
        _desktop_windows: &[DesktopWindow],
        hints: &HashMap<String, &DesktopWindow>,
    ) -> Result<()> {
        info!("Rendering {} hints", hints.len());
        Ok(())
    }

    pub fn wait_for_hint_selection<'a>(
        &mut self,
        hints: &HashMap<String, &'a DesktopWindow>,
    ) -> Result<Option<&'a DesktopWindow>> {
        let conn = Connection::connect_to_env().context("Failed to connect to Wayland")?;
        let (globals, mut event_queue) =
            registry_queue_init::<RenderState>(&conn).context("Failed to get global registry")?;
        let qh = event_queue.handle();

        let compositor: wl_compositor::WlCompositor = globals
            .bind(&qh, 4..=6, ())
            .context("Failed to bind wl_compositor")?;
        let shm: wl_shm::WlShm = globals
            .bind(&qh, 1..=1, ())
            .context("Failed to bind wl_shm")?;
        let layer_shell: zwlr_layer_shell_v1::ZwlrLayerShellV1 = globals
            .bind(&qh, 1..=4, ())
            .context("Failed to bind zwlr_layer_shell_v1")?;
        let seat: wl_seat::WlSeat = globals
            .bind(&qh, 1..=7, ())
            .context("Failed to bind wl_seat")?;

        // Bind every wl_output global so we can create one layer surface per monitor.
        // Bind at version 4 to receive the Name event for accurate monitor matching.
        // We store the WlOutput objects to keep them alive and to pass to get_layer_surface.
        struct OutputEntry {
            output: wl_output::WlOutput,
            data: Arc<Mutex<OutputData>>,
        }

        let output_entries: Vec<OutputEntry> = globals
            .contents()
            .clone_list()
            .into_iter()
            .filter(|g| g.interface == "wl_output")
            .map(|g| {
                let data = Arc::new(Mutex::new(OutputData::default()));
                let output: wl_output::WlOutput =
                    globals
                        .registry()
                        .bind(g.name, g.version.min(4), &qh, Arc::clone(&data));
                OutputEntry { output, data }
            })
            .collect();

        let total_surfaces = output_entries.len().max(1);

        let mut state = RenderState {
            _compositor: compositor.clone(),
            _shm: shm.clone(),
            _layer_shell: layer_shell.clone(),
            _seat: Some(seat.clone()),
            surfaces: (0..total_surfaces)
                .map(|_| SurfaceConfig {
                    width: 0,
                    height: 0,
                    offset_x: 0,
                    offset_y: 0,
                })
                .collect(),
            configured_count: 0,
            total_surfaces,
            keyboard_state: Some(KeyboardState {
                xkb_context: xkbcommon::xkb::Context::new(xkbcommon::xkb::CONTEXT_NO_FLAGS),
                xkb_state: None,
            }),
            pressed_keys: String::new(),
            should_exit: false,
        };

        // Roundtrip to receive wl_output::Geometry and Name events.
        event_queue.roundtrip(&mut state)?;

        // Get Hyprland monitor list – authoritative for window coordinate space and focus state.
        let hypr_monitors =
            crate::wm_hyprland::get_monitors().context("Failed to get Hyprland monitors")?;

        // Create one layer surface per output (or a single fallback surface).
        let mut wayland_surfaces: Vec<(
            wl_surface::WlSurface,
            zwlr_layer_surface_v1::ZwlrLayerSurfaceV1,
        )> = Vec::new();

        if output_entries.is_empty() {
            // No outputs enumerated – fall back to a single compositor-chosen surface.
            let surface = compositor.create_surface(&qh, ());
            let ls = layer_shell.get_layer_surface(
                &surface,
                None,
                Layer::Overlay,
                "hyprselect".to_string(),
                &qh,
                0usize,
            );
            ls.set_anchor(Anchor::Top | Anchor::Bottom | Anchor::Left | Anchor::Right);
            ls.set_keyboard_interactivity(KeyboardInteractivity::Exclusive);
            ls.set_exclusive_zone(-1);
            surface.commit();
            wayland_surfaces.push((surface, ls));
        } else {
            for (idx, entry) in output_entries.iter().enumerate() {
                let d = entry.data.lock().unwrap();
                let out_name = d.name.clone();
                let out_x = d.x;
                let out_y = d.y;
                drop(d);

                // Match this wl_output to a Hyprland monitor.
                // Primary: by name (requires wl_output v4). Fallback: by closest position.
                let hypr_mon = hypr_monitors
                    .iter()
                    .find(|m| !out_name.is_empty() && m.name == out_name)
                    .or_else(|| {
                        hypr_monitors
                            .iter()
                            .min_by_key(|m| (m.x - out_x).abs() + (m.y - out_y).abs())
                    });

                // Use Hyprland's coordinates as the authoritative offset and logical bounds.
                // These are in the same space as window positions from wm_hyprland.
                if let Some(m) = hypr_mon {
                    let logical_w = (m.width as f32 / m.scale).round() as i32;
                    let logical_h = (m.height as f32 / m.scale).round() as i32;
                    state.surfaces[idx].offset_x = m.x;
                    state.surfaces[idx].offset_y = m.y;
                    state.surfaces[idx].width = logical_w;
                    state.surfaces[idx].height = logical_h;
                } else {
                    state.surfaces[idx].offset_x = out_x;
                    state.surfaces[idx].offset_y = out_y;
                }

                let is_focused = hypr_mon.map_or(idx == 0, |m| m.focused);

                let surface = compositor.create_surface(&qh, ());
                let ls = layer_shell.get_layer_surface(
                    &surface,
                    Some(&entry.output),
                    Layer::Overlay,
                    "hyprselect".to_string(),
                    &qh,
                    idx,
                );
                ls.set_anchor(Anchor::Top | Anchor::Bottom | Anchor::Left | Anchor::Right);
                // Keyboard focus goes to the currently focused monitor's surface only.
                ls.set_keyboard_interactivity(if is_focused {
                    KeyboardInteractivity::Exclusive
                } else {
                    KeyboardInteractivity::None
                });
                ls.set_exclusive_zone(-1);
                surface.commit();
                wayland_surfaces.push((surface, ls));
            }
        }

        // Wait for every surface to receive its Configure event.
        while state.configured_count < state.total_surfaces {
            event_queue.blocking_dispatch(&mut state)?;
        }

        let _keyboard = seat.get_keyboard(&qh, ());

        // Render a separate buffer for each monitor, drawing only the hints
        // whose window center falls within that monitor's logical bounds.
        let mut buffers: Vec<wl_buffer::WlBuffer> = Vec::new();

        for (idx, (surface, _ls)) in wayland_surfaces.iter().enumerate() {
            let sc = &state.surfaces[idx];
            let width = if sc.width > 0 { sc.width } else { 1920 };
            let height = if sc.height > 0 { sc.height } else { 1080 };
            let offset_x = sc.offset_x;
            let offset_y = sc.offset_y;

            let monitor_hints: HashMap<String, &DesktopWindow> = hints
                .iter()
                .filter(|(_, w)| {
                    let cx = w.pos.0 + w.size.0 / 2;
                    let cy = w.pos.1 + w.size.1 / 2;
                    cx >= offset_x
                        && cx < offset_x + width
                        && cy >= offset_y
                        && cy < offset_y + height
                })
                .map(|(k, v)| (k.clone(), *v))
                .collect();

            let buffer = self.create_hints_buffer(&shm, &qh, sc, &monitor_hints)?;
            surface.attach(Some(&buffer), 0, 0);
            surface.damage_buffer(0, 0, width, height);
            surface.commit();
            buffers.push(buffer);
        }

        event_queue.roundtrip(&mut state)?;

        info!(
            "Overlay displayed on {} monitor(s). Press hint keys or ESC to cancel.",
            wayland_surfaces.len()
        );

        while !state.should_exit {
            event_queue.blocking_dispatch(&mut state)?;

            if let Some(window) = hints.get(&state.pressed_keys) {
                info!("Hint '{}' selected", state.pressed_keys);
                return Ok(Some(window));
            }
        }

        Ok(None)
    }

    fn create_hints_buffer(
        &self,
        shm: &wl_shm::WlShm,
        qh: &QueueHandle<RenderState>,
        sc: &SurfaceConfig,
        hints: &HashMap<String, &DesktopWindow>,
    ) -> Result<wl_buffer::WlBuffer> {
        let width = if sc.width > 0 { sc.width } else { 1920 };
        let height = if sc.height > 0 { sc.height } else { 1080 };
        let stride = width * 4;
        let size = stride * height;

        let temp_file = tempfile::tempfile().context("Failed to create temp file")?;
        temp_file
            .set_len(size as u64)
            .context("Failed to set file size")?;

        let mut cairo_surface = cairo::ImageSurface::create(cairo::Format::ARgb32, width, height)
            .context("Failed to create Cairo surface")?;

        {
            let cairo_context =
                cairo::Context::new(&cairo_surface).context("Failed to create Cairo context")?;

            cairo_context.set_source_rgba(0.0, 0.0, 0.0, 0.0);
            cairo_context
                .paint()
                .context("Failed to paint background")?;

            for (hint, window) in hints {
                self.draw_hint(&cairo_context, hint, window, sc.offset_x, sc.offset_y)?;
            }
        }

        cairo_surface.flush();
        let cairo_data = cairo_surface.data().context("Failed to get Cairo data")?;

        let mut mmap = unsafe { memmap2::MmapMut::map_mut(&temp_file).context("mmap failed")? };
        mmap.copy_from_slice(&cairo_data);
        drop(mmap);

        let pool = shm.create_pool(temp_file.as_fd(), size, qh, ());
        let buffer = pool.create_buffer(0, width, height, stride, wl_shm::Format::Argb8888, qh, ());
        pool.destroy();
        Ok(buffer)
    }

    fn draw_hint(
        &self,
        ctx: &cairo::Context,
        hint: &str,
        window: &DesktopWindow,
        offset_x: i32,
        offset_y: i32,
    ) -> Result<()> {
        use crate::args::{HorizontalAlign, VerticalAlign};

        ctx.select_font_face(
            &self.app_config.font.font_family,
            cairo::FontSlant::Normal,
            cairo::FontWeight::Bold,
        );
        ctx.set_font_size(self.app_config.font.font_size);

        let text_extents = ctx.text_extents(hint)?;

        let base_size = self.app_config.font.font_size;
        let margin_left = base_size * self.app_config.margin.left as f64;
        let margin_right = base_size * self.app_config.margin.right as f64;
        let margin_top = base_size * self.app_config.margin.top as f64;
        let margin_bottom = base_size * self.app_config.margin.bottom as f64;

        // Translate to monitor-local coordinates.
        let local_x = (window.pos.0 - offset_x) as f64;
        let local_y = (window.pos.1 - offset_y) as f64;

        let (rect_width, rect_height, x, y) = if self.app_config.fill {
            (window.size.0 as f64, window.size.1 as f64, local_x, local_y)
        } else {
            let rw = text_extents.width() + margin_left + margin_right;
            let rh = base_size + margin_top + margin_bottom;

            let x_offset = self.app_config.offset.x as f64;
            let rx = match self.app_config.horizontal_align {
                HorizontalAlign::Left => local_x + x_offset,
                HorizontalAlign::Center => local_x + (window.size.0 as f64 - rw) / 2.0,
                HorizontalAlign::Right => local_x + window.size.0 as f64 - rw - x_offset,
            };

            let y_offset = self.app_config.offset.y as f64;
            let ry = match self.app_config.vertical_align {
                VerticalAlign::Top => local_y + y_offset,
                VerticalAlign::Center => local_y + (window.size.1 as f64 - rh) / 2.0,
                VerticalAlign::Bottom => local_y + window.size.1 as f64 - rh - y_offset,
            };

            (rw, rh, rx, ry)
        };

        let bg = if window.is_focused {
            self.app_config.bg_color_current
        } else {
            self.app_config.bg_color
        };
        ctx.set_source_rgba(bg.0, bg.1, bg.2, bg.3);

        let radius = if self.app_config.fill { 0.0 } else { 5.0 };
        let degrees = std::f64::consts::PI / 180.0;

        ctx.new_sub_path();
        ctx.arc(
            x + rect_width - radius,
            y + radius,
            radius,
            -90.0 * degrees,
            0.0 * degrees,
        );
        ctx.arc(
            x + rect_width - radius,
            y + rect_height - radius,
            radius,
            0.0 * degrees,
            90.0 * degrees,
        );
        ctx.arc(
            x + radius,
            y + rect_height - radius,
            radius,
            90.0 * degrees,
            180.0 * degrees,
        );
        ctx.arc(
            x + radius,
            y + radius,
            radius,
            180.0 * degrees,
            270.0 * degrees,
        );
        ctx.close_path();
        ctx.fill()?;

        let text = if window.is_focused {
            self.app_config.text_color_current
        } else {
            self.app_config.text_color
        };
        ctx.set_source_rgba(text.0, text.1, text.2, text.3);

        let text_x = x + (rect_width - text_extents.width()) / 2.0 - text_extents.x_bearing();
        let text_y = y + (rect_height - text_extents.height()) / 2.0 - text_extents.y_bearing();
        ctx.move_to(text_x, text_y);
        ctx.show_text(hint)?;

        Ok(())
    }
}
