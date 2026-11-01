//! Native backend: DRM/KMS output, libinput, libseat session. Single GPU, first connected monitor.

use std::{cell::RefCell, collections::HashMap, rc::Rc, time::Duration};

use smithay::{
    backend::input::InputEvent,
    backend::{
        allocator::{
            format::FormatSet,
            gbm::{GbmAllocator, GbmBufferFlags, GbmDevice},
            Fourcc, Modifier,
        },
        drm::{
            compositor::FrameFlags,
            exporter::gbm::GbmFramebufferExporter,
            output::{DrmOutput, DrmOutputManager, DrmOutputRenderElements},
            DrmDevice, DrmDeviceFd, DrmEvent, DrmNode, NodeType,
        },
        egl::{EGLContext, EGLDisplay},
        libinput::{LibinputInputBackend, LibinputSessionInterface},
        renderer::{gles::GlesRenderer, ImportDma, ImportMemWl},
        session::{
            libseat::LibSeatSession,
            Event as SessionEvent, Session,
        },
        udev::{all_gpus, primary_gpu, UdevBackend, UdevEvent},
    },
    output::{Mode as WlMode, Output, PhysicalProperties},
    reexports::{
        calloop::{
            timer::{TimeoutAction, Timer},
            EventLoop,
        },
        drm::control::{connector, crtc, ModeTypeFlags},
        input::Libinput,
        rustix::fs::OFlags,
    },
    utils::DeviceFd,
    wayland::dmabuf::DmabufFeedbackBuilder,
};
use smithay_drm_extras::{
    display_info,
    drm_scanner::{DrmScanEvent, DrmScanner},
};

use crate::{render::SekuroElement, state::SekuroState};

type Alloc = GbmAllocator<DrmDeviceFd>;
type Exporter = GbmFramebufferExporter<DrmDeviceFd>;
type Manager = DrmOutputManager<Alloc, Exporter, (), DrmDeviceFd>;
type OutputHandle = DrmOutput<Alloc, Exporter, (), DrmDeviceFd>;

const COLOR_FORMATS: &[Fourcc] = &[Fourcc::Abgr8888, Fourcc::Argb8888];

struct Surface {
    drm_output: OutputHandle,
    output: Output,
    connector: connector::Handle,
    /// When the last queued frame was submitted; `Some` until its vblank arrives.
    pending_since: Option<std::time::Instant>,
}

struct Drm {
    manager: Manager,
    scanner: DrmScanner,
    surfaces: HashMap<crtc::Handle, Surface>,
    node: DrmNode,
}

type Shared = Rc<RefCell<Option<Drm>>>;

pub fn init(event_loop: &mut EventLoop<'static, SekuroState>, state: &mut SekuroState) -> Result<(), String> {
    let (session, notifier) = LibSeatSession::new().map_err(|e| format!("could not open a session (libseat): {e}"))?;
    let seat = session.seat();

    let udev = UdevBackend::new(&seat).map_err(|e| format!("udev: {e}"))?;

    let gpu_path = primary_gpu(&seat)
        .ok()
        .flatten()
        .or_else(|| all_gpus(&seat).ok().and_then(|g| g.into_iter().next()))
        .ok_or("no GPU found")?;
    let node = DrmNode::from_path(&gpu_path).map_err(|e| format!("drm node: {e}"))?;
    let primary_node = node.node_with_type(NodeType::Primary).and_then(|n| n.ok()).unwrap_or(node);
    tracing::info!("using GPU {}", gpu_path.display());

    // libinput
    let mut libinput = Libinput::new_with_udev::<LibinputSessionInterface<LibSeatSession>>(session.clone().into());
    libinput.udev_assign_seat(&seat).map_err(|_| "libinput: could not assign seat".to_string())?;
    let libinput_backend = LibinputInputBackend::new(libinput.clone());
    event_loop
        .handle()
        .insert_source(libinput_backend, |event, _, state| {
            if let InputEvent::DeviceAdded { device } = &event {
                let mut device = device.clone();
                state.configure_device(&mut device);
                state.input_devices.push(device);
            } else if let InputEvent::DeviceRemoved { device } = &event {
                state.input_devices.retain(|d| d != device);
            }
            state.process_input_event(event)
        })
        .map_err(|e| e.to_string())?;

    // Open the DRM device through the session so we don't need root.
    let fd = session
        .clone()
        .open(&gpu_path, OFlags::RDWR | OFlags::CLOEXEC | OFlags::NOCTTY | OFlags::NONBLOCK)
        .map_err(|e| format!("open {}: {e}", gpu_path.display()))?;
    let fd = DrmDeviceFd::new(DeviceFd::from(fd));
    let (drm, drm_notifier) = DrmDevice::new(fd.clone(), true).map_err(|e| format!("drm: {e}"))?;
    let gbm = GbmDevice::new(fd).map_err(|e| format!("gbm: {e}"))?;

    // Renderer: GLES via EGL on GBM. Mesa falls back to llvmpipe in virtual machines.
    let egl_display = unsafe { EGLDisplay::new(gbm.clone()) }.map_err(|e| format!("egl display: {e}"))?;
    let egl_context = EGLContext::new(&egl_display).map_err(|e| format!("egl context: {e}"))?;
    let mut renderer = unsafe { GlesRenderer::new(egl_context) }.map_err(|e| format!("gles renderer: {e}"))?;

    let render_formats: FormatSet = renderer
        .egl_context()
        .dmabuf_render_formats()
        .iter()
        .copied()
        .collect::<Vec<_>>()
        .into_iter()
        .filter(|f| COLOR_FORMATS.contains(&f.code) || f.modifier == Modifier::Linear)
        .collect();
    let _ = &render_formats;

    state.shm_state.update_formats(renderer.shm_formats());
    let dmabuf_formats = renderer.dmabuf_formats();
    if let Ok(feedback) = DmabufFeedbackBuilder::new(primary_node.dev_id(), dmabuf_formats).build() {
        let _global = state.dmabuf_state.create_global_with_default_feedback::<SekuroState>(&state.display_handle, &feedback);
    }

    let allocator = GbmAllocator::new(gbm.clone(), GbmBufferFlags::RENDERING | GbmBufferFlags::SCANOUT);
    let exporter = GbmFramebufferExporter::new(gbm.clone(), Some(node).into());
    let manager = DrmOutputManager::new(
        drm,
        allocator,
        exporter,
        Some(gbm),
        COLOR_FORMATS.iter().copied(),
        renderer.egl_context().dmabuf_render_formats().iter().copied(),
    );

    state.shaders = crate::render::Shaders::compile(&mut renderer);
    state.gpu = Some(renderer);
    state.session = Some(session.clone());

    let shared: Shared = Rc::new(RefCell::new(Some(Drm {
        manager,
        scanner: DrmScanner::new(),
        surfaces: HashMap::new(),
        node,
    })));

    // VBlank events: the previous frame hit the screen, draw the next one.
    {
        let shared = shared.clone();
        event_loop
            .handle()
            .insert_source(drm_notifier, move |event, _, state| match event {
                DrmEvent::VBlank(crtc) => {
                    if let Some(drm) = shared.borrow_mut().as_mut() {
                        if let Some(surface) = drm.surfaces.get_mut(&crtc) {
                            let _ = surface.drm_output.frame_submitted();
                            surface.pending_since = None;
                        }
                    }
                    render(state, &shared, crtc);
                }
                DrmEvent::Error(e) => tracing::error!("drm error: {e}"),
            })
            .map_err(|e| e.to_string())?;
    }

    // VT switching: stop drawing when we lose the seat, resume when we get it back.
    {
        let shared = shared.clone();
        let mut libinput = libinput;
        event_loop
            .handle()
            .insert_source(notifier, move |event, _, state| match event {
                SessionEvent::PauseSession => {
                    libinput.suspend();
                    if let Some(drm) = shared.borrow_mut().as_mut() {
                        drm.manager.pause();
                    }
                }
                SessionEvent::ActivateSession => {
                    if libinput.resume().is_err() {
                        tracing::error!("failed to resume libinput");
                    }
                    let crtcs: Vec<_> = {
                        let mut guard = shared.borrow_mut();
                        let Some(drm) = guard.as_mut() else { return };
                        if let Err(e) = drm.manager.lock().activate(false) {
                            tracing::error!("failed to re-activate drm: {e}");
                        }
                        drm.surfaces.keys().copied().collect()
                    };
                    for crtc in crtcs {
                        let shared = shared.clone();
                        state.loop_handle.insert_idle(move |state| render(state, &shared, crtc));
                    }
                }
            })
            .map_err(|e| e.to_string())?;
    }

    // Hotplug.
    {
        let shared = shared.clone();
        event_loop
            .handle()
            .insert_source(udev, move |event, _, state| {
                if let UdevEvent::Changed { device_id } = event {
                    let is_ours = shared.borrow().as_ref().is_some_and(|d| d.node.dev_id() == device_id);
                    if is_ours {
                        scan_connectors(state, &shared);
                        state.mode_dirty = true;
                    }
                }
            })
            .map_err(|e| e.to_string())?;
    }

    // Re-evaluate the display mode when the setting changes or the monitor (or a VM window) is resized.
    {
        let shared = shared.clone();
        event_loop
            .handle()
            .insert_source(Timer::from_duration(Duration::from_millis(500)), move |_, _, state| {
                let crtcs: Vec<crtc::Handle> =
                    shared.borrow().as_ref().map(|d| d.surfaces.keys().copied().collect()).unwrap_or_default();
                for crtc in crtcs {
                    apply_display_mode(state, &shared, crtc);
                }
                state.check_idle();
                TimeoutAction::ToDuration(Duration::from_millis(2000))
            })
            .map_err(|e| e.to_string())?;
    }

    scan_connectors(state, &shared);
    if state.output.is_none() {
        return Err("no connected display found".into());
    }
    Ok(())
}

fn scan_connectors(state: &mut SekuroState, shared: &Shared) {
    let events: Vec<DrmScanEvent> = {
        let mut guard = shared.borrow_mut();
        let Some(drm) = guard.as_mut() else { return };
        let Drm { scanner, manager, .. } = drm;
        match scanner.scan_connectors(manager.device()) {
            Ok(result) => result.into_iter().collect(),
            Err(e) => {
                tracing::warn!("connector scan failed: {e}");
                return;
            }
        }
    };
    for event in events {
        match event {
            DrmScanEvent::Connected { connector, crtc: Some(crtc) } => connector_connected(state, shared, connector, crtc),
            DrmScanEvent::Disconnected { crtc: Some(crtc), .. } => connector_disconnected(state, shared, crtc),
            _ => {}
        }
    }
}

fn connector_connected(state: &mut SekuroState, shared: &Shared, connector: connector::Info, crtc: crtc::Handle) {
    if connector.modes().is_empty() || (!state.outputs.is_empty() && !state.config.display.extra_monitors) {
        return;
    }
    let name = format!("{}-{}", connector.interface().as_str(), connector.interface_id());
    tracing::info!("enabling output {name}");

    let mut guard = shared.borrow_mut();
    let Some(drm) = guard.as_mut() else { return };

    let info = display_info::for_connector(drm.manager.device(), connector.handle());
    let make = info.as_ref().and_then(|i| i.make()).unwrap_or_else(|| "Unknown".into());
    let model = info.as_ref().and_then(|i| i.model()).unwrap_or_else(|| "Unknown".into());

    let mode = connector
        .modes()
        .iter()
        .find(|m| m.mode_type().contains(ModeTypeFlags::PREFERRED))
        .copied()
        .unwrap_or(connector.modes()[0]);
    let wl_mode = WlMode::from(mode);
    let (pw, ph) = connector.size().unwrap_or((0, 0));
    let output = Output::new(
        name,
        PhysicalProperties {
            size: (pw as i32, ph as i32).into(),
            subpixel: connector.subpixel().into(),
            make,
            model,
            serial_number: "Unknown".into(),
        },
    );
    let _global = output.create_global::<SekuroState>(&state.display_handle);
    output.set_preferred(wl_mode);
    let scale = sekuro_common::config::scale_for(&state.config.display.scale, wl_mode.size.w as u32);
    output.change_current_state(Some(wl_mode), None, Some(smithay::output::Scale::Integer(scale)), Some((0, 0).into()));

    // Compose everything on the primary plane. Hardware overlay planes (and direct scan-out of client
    // buffers) are an optimisation that misbehaves on virtual GPUs, leaving stale areas on screen.
    let mut planes = drm.manager.device().planes(&crtc).ok();
    if let Some(p) = planes.as_mut() {
        if std::env::var_os("SEKURO_OVERLAY").is_none() {
            p.overlay.clear();
        }
    }
    let Some(mut renderer) = state.gpu.take() else { return };
    let result = drm.manager.lock().initialize_output::<_, SekuroElement>(
        crtc,
        mode,
        &[connector.handle()],
        &output,
        planes,
        &mut renderer,
        &DrmOutputRenderElements::default(),
    );
    state.gpu = Some(renderer);

    match result {
        Ok(drm_output) => {
            drm.surfaces.insert(crtc, Surface { drm_output, output: output.clone(), connector: connector.handle(), pending_since: None });
            drop(guard);
            let name = output.name();
            let secondary = !state.outputs.is_empty();
            state.add_output(output);
            state.reload_cursor();
            if secondary {
                state.spawn_bar_on(&name);
            }
            let shared = shared.clone();
            state.loop_handle.insert_idle(move |state| render(state, &shared, crtc));
        }
        Err(e) => tracing::error!("failed to initialise output: {e}"),
    }
}

fn connector_disconnected(state: &mut SekuroState, shared: &Shared, crtc: crtc::Handle) {
    let removed = shared.borrow_mut().as_mut().and_then(|d| d.surfaces.remove(&crtc));
    if let Some(surface) = removed {
        // Close the bar (and anything else) that lived on this monitor.
        for layer in smithay::desktop::layer_map_for_output(&surface.output).layers() {
            layer.layer_surface().send_close();
        }
        state.remove_output(&surface.output);
        state.reload_cursor();
    }
}

/// Draw one frame on `crtc`; the next frame is triggered by the vblank (or a timer when idle).
fn render(state: &mut SekuroState, shared: &Shared, crtc: crtc::Handle) {
    let output = {
        let guard = shared.borrow();
        match guard.as_ref().and_then(|d| d.surfaces.get(&crtc)) {
            Some(s) => s.output.clone(),
            None => return,
        }
    };
    // Partial-damage repaint leaves stale areas on some virtual GPUs, so by default every frame that is
    // drawn is drawn in full, and frames are only drawn when something actually changed.
    let partial = std::env::var_os("SEKURO_PARTIAL_DAMAGE").is_some();
    let waiting = {
        let mut guard = shared.borrow_mut();
        match guard.as_mut().and_then(|d| d.surfaces.get_mut(&crtc)) {
            Some(surface) => {
                if surface.pending_since.is_some_and(|t| t.elapsed() > Duration::from_millis(300)) {
                    surface.pending_since = None; // a lost vblank must not freeze the screen
                }
                surface.pending_since.is_some()
            }
            None => return,
        }
    };
    if waiting || (!partial && !state.needs_render) {
        state.send_frames(&output);
        reschedule(state, shared, crtc);
        return;
    }
    if !partial {
        state.needs_render = false;
    }

    let Some(mut renderer) = state.gpu.take() else { return };

    let elements = state.build_elements(&mut renderer, &output);
    let clear = state.clear_color();

    let rendered = {
        let mut guard = shared.borrow_mut();
        let Some(surface) = guard.as_mut().and_then(|d| d.surfaces.get_mut(&crtc)) else {
            state.gpu = Some(renderer);
            return;
        };
        if !partial {
            surface.drm_output.reset_buffers();
        }
        let has_damage = match surface.drm_output.render_frame(&mut renderer, &elements, clear, frame_flags()) {
            Ok(result) => {
                tracing::debug!("rendered {} elements, empty={}", elements.len(), result.is_empty);
                !result.is_empty
            }
            Err(e) => {
                tracing::warn!("render error: {e:?}");
                false
            }
        };
        if has_damage {
            match surface.drm_output.queue_frame(()) {
                Ok(()) => {
                    tracing::debug!("frame queued");
                    surface.pending_since = Some(std::time::Instant::now());
                    true
                }
                Err(e) => {
                    tracing::warn!("queue_frame: {e:?}");
                    false
                }
            }
        } else {
            false
        }
    };
    drop(elements);
    state.gpu = Some(renderer);
    state.send_frames(&output);

    if !rendered {
        // Nothing changed (or a transient error): look again in about one frame.
        reschedule(state, shared, crtc);
    }
}

fn reschedule(state: &mut SekuroState, shared: &Shared, crtc: crtc::Handle) {
    let shared = shared.clone();
    let _ = state.loop_handle.insert_source(Timer::from_duration(Duration::from_millis(16)), move |_, _, state| {
        render(state, &shared, crtc);
        TimeoutAction::Drop
    });
}

/// Pick the mode requested by `display.mode` (or the preferred one) and switch to it if it differs.
fn apply_display_mode(state: &mut SekuroState, shared: &Shared, crtc: crtc::Handle) {
    use smithay::reexports::drm::control::Device as _;
    use sekuro_common::ipc::ModeInfo;

    let (connector, output) = {
        let guard = shared.borrow();
        let Some(surface) = guard.as_ref().and_then(|d| d.surfaces.get(&crtc)) else { return };
        (surface.connector, surface.output.clone())
    };
    let modes = {
        let guard = shared.borrow();
        let Some(drm) = guard.as_ref() else { return };
        match drm.manager.device().get_connector(connector, false) {
            Ok(info) => info.modes().to_vec(),
            Err(_) => return,
        }
    };
    if modes.is_empty() {
        return;
    }

    // Publish the list (de-duplicated by size and refresh) for the settings app.
    let mut infos: Vec<ModeInfo> = Vec::new();
    for m in &modes {
        let (w, h) = m.size();
        let info = ModeInfo {
            width: w as u32,
            height: h as u32,
            refresh_mhz: WlMode::from(*m).refresh as u32,
            preferred: m.mode_type().contains(ModeTypeFlags::PREFERRED),
        };
        if !infos.iter().any(|i| i.width == info.width && i.height == info.height && i.refresh_mhz == info.refresh_mhz) {
            infos.push(info);
        }
    }
    // The mode list and the chosen mode in the settings belong to the primary monitor.
    let is_primary = state.output.as_ref() == Some(&output);
    let list_changed = is_primary && state.output_modes != infos;
    if is_primary {
        state.output_modes = infos;
    }

    let wanted = if is_primary { sekuro_common::config::parse_mode(&state.config.display.mode) } else { None };
    let target = match wanted {
        Some((w, h, hz)) => modes
            .iter()
            .filter(|m| m.size() == (w as u16, h as u16))
            .min_by_key(|m| {
                let r = WlMode::from(**m).refresh as f64 / 1000.0;
                hz.map(|hz| ((r - hz).abs() * 100.0) as i64).unwrap_or(-(r as i64))
            })
            .copied(),
        None => None,
    }
    .or_else(|| modes.iter().find(|m| m.mode_type().contains(ModeTypeFlags::PREFERRED)).copied())
    .unwrap_or(modes[0]);

    let target_wl = WlMode::from(target);
    let want_scale = sekuro_common::config::scale_for(&state.config.display.scale, target_wl.size.w as u32);
    let scale_changed = output.current_scale().integer_scale() != want_scale;
    let changed = output.current_mode().map_or(true, |c| c.size != target_wl.size || (wanted.is_some() && c.refresh != target_wl.refresh));
    if !changed && scale_changed {
        output.change_current_state(None, None, Some(smithay::output::Scale::Integer(want_scale)), None);
        state.layout_outputs();
        state.reload_cursor();
        state.relayout();
        state.mode_dirty = false;
        return;
    }
    if !changed {
        if list_changed {
            state.mode_dirty = false;
        }
        return;
    }

    let Some(mut renderer) = state.gpu.take() else { return };
    let result = {
        let mut guard = shared.borrow_mut();
        let Some(surface) = guard.as_mut().and_then(|d| d.surfaces.get_mut(&crtc)) else {
            state.gpu = Some(renderer);
            return;
        };
        surface.drm_output.use_mode::<_, SekuroElement>(target, &mut renderer, &DrmOutputRenderElements::default())
    };
    state.gpu = Some(renderer);
    match result {
        Ok(()) => {
            tracing::info!("display mode set to {}x{}", target_wl.size.w, target_wl.size.h);
            output.change_current_state(Some(target_wl), None, Some(smithay::output::Scale::Integer(want_scale)), None);
            output.set_preferred(target_wl);
            state.layout_outputs();
            if state.output_at(state.pointer_location).is_none() {
                if let Some(geo) = state.output.as_ref().and_then(|o| state.space.output_geometry(o)) {
                    state.pointer_location = (geo.loc.x as f64 + geo.size.w as f64 / 2.0, geo.loc.y as f64 + geo.size.h as f64 / 2.0).into();
                }
            }
            state.reload_cursor();
            state.relayout();
            state.mode_dirty = false;
        }
        Err(e) => tracing::warn!("could not switch display mode: {e:?}"),
    }
}

/// Frame composition flags. `SEKURO_PLANES=1` re-enables hardware plane scan-out for experiments.
fn frame_flags() -> FrameFlags {
    if std::env::var_os("SEKURO_PLANES").is_some() { FrameFlags::DEFAULT } else { FrameFlags::empty() }
}
