//! Nested backend: runs Steady inside a window on another compositor. Used for development.

use smithay::{
    backend::{
        allocator::Fourcc,
        renderer::{damage::OutputDamageTracker, gles::GlesRenderer, ExportMem},
        winit::{self, WinitEvent},
    },
    output::{Mode, Output, PhysicalProperties, Subpixel},
    reexports::calloop::EventLoop,
    utils::{Buffer, Physical, Rectangle, Size, Transform},
};

use crate::state::SteadyState;

pub static REDRAWS: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);

pub fn init(event_loop: &mut EventLoop<'static, SteadyState>, state: &mut SteadyState) -> Result<(), String> {
    let (mut backend, winit_source) = winit::init::<GlesRenderer>().map_err(|e| format!("winit: {e:?}"))?;

    let mode = Mode { size: backend.window_size(), refresh: 60_000 };
    let output = Output::new(
        "steady-nested".to_string(),
        PhysicalProperties {
            size: (0, 0).into(),
            subpixel: Subpixel::Unknown,
            make: "Steady".into(),
            model: "Nested".into(),
            serial_number: "0".into(),
        },
    );
    let _global = output.create_global::<SteadyState>(&state.display_handle);
    output.change_current_state(Some(mode), Some(Transform::Flipped180), None, Some((0, 0).into()));
    output.set_preferred(mode);
    state.add_output(output.clone());

    state.shaders = crate::render::Shaders::compile(backend.renderer());
    state.fb_flip = std::env::var_os("STEADY_NO_FB_FLIP").is_none();
    let mut damage_tracker = OutputDamageTracker::from_output(&output);
    let mut frames = 0u32;
    let mut last_log = std::time::Instant::now();

    event_loop
        .handle()
        .insert_source(winit_source, move |event, _, state| match event {
            WinitEvent::Resized { size, .. } => {
                output.change_current_state(Some(Mode { size, refresh: 60_000 }), None, None, None);
                state.relayout();
            }
            WinitEvent::Input(event) => state.process_input_event(event),
            WinitEvent::Redraw => {
                frames += 1;
                REDRAWS.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                if last_log.elapsed().as_secs() >= 2 {
                    tracing::debug!("redraws in last 2s: {frames}");
                    frames = 0;
                    last_log = std::time::Instant::now();
                }
                let clear = state.clear_color();
                let backend_size = backend.window_size();
                let age = if std::env::var_os("STEADY_FULL_REPAINT").is_some() { 0 } else { backend.buffer_age().unwrap_or(0) };
                let drew = match backend.bind() {
                    Ok((renderer, mut framebuffer)) => {
                        let elements = state.build_elements(renderer, &output);
                        if let Err(e) = damage_tracker.render_output(renderer, &mut framebuffer, age, &elements, clear) {
                            tracing::warn!("render: {e:?}");
                        }
                        if let Some(path) = state.screenshot_request.take() {
                            if let Err(e) = save_png(renderer, &framebuffer, backend_size, &path) {
                                tracing::warn!("screenshot: {e}");
                            }
                        }
                        true
                    }
                    Err(e) => {
                        tracing::warn!("bind failed: {e:?}");
                        false
                    }
                };
                if drew {
                    let _ = backend.submit(None);
                    state.send_frames(&output);
                }
                // Always ask for the next frame, even after a failure, or the loop dies.
                backend.window().request_redraw();
            }
            WinitEvent::CloseRequested => state.loop_signal.stop(),
            _ => {}
        })
        .map_err(|e| e.to_string())?;
    Ok(())
}

/// Development aid: dump the framebuffer as a PNG.
fn save_png(
    renderer: &mut GlesRenderer,
    framebuffer: &smithay::backend::renderer::gles::GlesTarget<'_>,
    size: Size<i32, Physical>,
    path: &str,
) -> Result<(), String> {
    let region = Rectangle::<i32, Buffer>::from_size((size.w, size.h).into());
    let mapping = renderer.copy_framebuffer(framebuffer, region, Fourcc::Abgr8888).map_err(|e| format!("{e:?}"))?;
    let bytes = renderer.map_texture(&mapping).map_err(|e| format!("{e:?}"))?;
    let (w, h) = (size.w as usize, size.h as usize);
    // GL reads bottom-up; PNG is top-down.
    let mut flipped = Vec::with_capacity(bytes.len());
    for row in (0..h).rev() {
        flipped.extend_from_slice(&bytes[row * w * 4..(row + 1) * w * 4]);
    }
    let file = std::fs::File::create(path).map_err(|e| e.to_string())?;
    let mut enc = png::Encoder::new(std::io::BufWriter::new(file), w as u32, h as u32);
    enc.set_color(png::ColorType::Rgba);
    enc.set_depth(png::BitDepth::Eight);
    enc.write_header().and_then(|mut wr| wr.write_image_data(&flipped)).map_err(|e| e.to_string())
}
