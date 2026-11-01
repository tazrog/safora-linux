//! Scene assembly shared by the DRM and nested (winit) backends.

use std::{io::Read, sync::Mutex, time::Duration};

use smithay::{
    backend::{
        allocator::Fourcc,
        renderer::{
            element::{
                memory::{MemoryRenderBuffer, MemoryRenderBufferRenderElement},
                solid::SolidColorRenderElement,
                surface::{render_elements_from_surface_tree, WaylandSurfaceRenderElement},
                Element, Id, Kind, RenderElement, UnderlyingStorage,
            },
            gles::{
                element::PixelShaderElement, GlesError, GlesFrame, GlesPixelProgram, GlesRenderer, GlesTexProgram,
                Uniform, UniformName, UniformType,
            },
            utils::{CommitCounter, DamageSet, OpaqueRegions},
            Color32F,
        },
    },
    desktop::{layer_map_for_output, utils::send_frames_surface_tree, PopupManager, Window},
    input::pointer::{CursorImageAttributes, CursorImageStatus},
    output::Output,
    render_elements,
    utils::{Buffer, Logical, Physical, Point, Rectangle, Scale, Transform},
    wayland::{compositor::with_states, shell::wlr_layer::Layer},
};
use safora_common::config::{parse_color, WallpaperMode};

use crate::state::SaforaState;

render_elements! {
    pub SaforaElement<=GlesRenderer>;
    Surface=WaylandSurfaceRenderElement<GlesRenderer>,
    Rounded=RoundedSurface,
    Solid=SolidColorRenderElement,
    Pixel=PixelShaderElement,
    Memory=MemoryRenderBufferRenderElement<GlesRenderer>,
}

impl std::fmt::Debug for SaforaElement {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("SaforaElement")
    }
}

// ------------------------------------------------------------------------------------------ shaders

const ROUND_TEX_SRC: &str = r#"
//_DEFINES_
precision highp float;
uniform sampler2D tex;
uniform float alpha;
varying vec2 v_coords;
uniform vec4 rect;      // window rectangle in framebuffer pixels: x, y, w, h
uniform float radius;
void main() {
    vec4 color = texture2D(tex, v_coords);
#if defined(NO_ALPHA)
    color = vec4(color.rgb, 1.0);
#endif
    vec2 half_size = rect.zw * 0.5;
    vec2 p = gl_FragCoord.xy - (rect.xy + half_size);
    vec2 q = abs(p) - half_size + radius;
    float d = length(max(q, 0.0)) + min(max(q.x, q.y), 0.0) - radius;
    float cover = clamp(0.5 - d, 0.0, 1.0);
    gl_FragColor = color * alpha * cover;
}
"#;

const RING_SRC: &str = r#"
precision highp float;
uniform vec2 size;
uniform float alpha;
varying vec2 v_coords;
uniform vec4 color;     // premultiplied
uniform float radius;   // inner (window) corner radius
uniform float bw;       // border width
void main() {
    vec2 p = (v_coords - 0.5) * size;
    vec2 hs = size * 0.5;
    float r_out = radius + bw;
    vec2 q = abs(p) - hs + r_out;
    float d_out = length(max(q, 0.0)) + min(max(q.x, q.y), 0.0) - r_out;
    vec2 q2 = abs(p) - (hs - bw) + radius;
    float d_in = length(max(q2, 0.0)) + min(max(q2.x, q2.y), 0.0) - radius;
    float a = clamp(0.5 - d_out, 0.0, 1.0) * (1.0 - clamp(0.5 - d_in, 0.0, 1.0));
    gl_FragColor = color * a * alpha;
}
"#;

#[derive(Clone)]
pub struct Shaders {
    round_tex: GlesTexProgram,
    ring: GlesPixelProgram,
}

impl Shaders {
    pub fn compile(renderer: &mut GlesRenderer) -> Option<Self> {
        let round_tex = renderer
            .compile_custom_texture_shader(
                ROUND_TEX_SRC,
                &[UniformName::new("rect", UniformType::_4f), UniformName::new("radius", UniformType::_1f)],
            )
            .map_err(|e| tracing::warn!("rounded-corner shader failed: {e:?}"))
            .ok()?;
        let ring = renderer
            .compile_custom_pixel_shader(
                RING_SRC,
                &[
                    UniformName::new("color", UniformType::_4f),
                    UniformName::new("radius", UniformType::_1f),
                    UniformName::new("bw", UniformType::_1f),
                ],
            )
            .map_err(|e| tracing::warn!("border shader failed: {e:?}"))
            .ok()?;
        Some(Self { round_tex, ring })
    }
}

/// A surface drawn with its corners clipped to a rounded rectangle.
#[derive(Debug)]
pub struct RoundedSurface {
    inner: WaylandSurfaceRenderElement<GlesRenderer>,
    program: GlesTexProgram,
    rect: [f32; 4],
    radius: f32,
}

impl Element for RoundedSurface {
    fn id(&self) -> &Id {
        self.inner.id()
    }
    fn current_commit(&self) -> CommitCounter {
        self.inner.current_commit()
    }
    fn location(&self, scale: Scale<f64>) -> Point<i32, Physical> {
        self.inner.location(scale)
    }
    fn src(&self) -> Rectangle<f64, Buffer> {
        self.inner.src()
    }
    fn transform(&self) -> Transform {
        self.inner.transform()
    }
    fn geometry(&self, scale: Scale<f64>) -> Rectangle<i32, Physical> {
        self.inner.geometry(scale)
    }
    fn damage_since(&self, scale: Scale<f64>, commit: Option<CommitCounter>) -> DamageSet<i32, Physical> {
        self.inner.damage_since(scale, commit)
    }
    // The clipped corners are transparent, so nothing may be treated as opaque.
    fn opaque_regions(&self, _scale: Scale<f64>) -> OpaqueRegions<i32, Physical> {
        OpaqueRegions::default()
    }
    fn alpha(&self) -> f32 {
        self.inner.alpha()
    }
    fn kind(&self) -> Kind {
        self.inner.kind()
    }
}

impl RenderElement<GlesRenderer> for RoundedSurface {
    fn draw(
        &self,
        frame: &mut GlesFrame<'_, '_>,
        src: Rectangle<f64, Buffer>,
        dst: Rectangle<i32, Physical>,
        damage: &[Rectangle<i32, Physical>],
        opaque_regions: &[Rectangle<i32, Physical>],
        cache: Option<&smithay::utils::user_data::UserDataMap>,
    ) -> Result<(), GlesError> {
        let [x, y, w, h] = self.rect;
        frame.override_default_tex_program(
            self.program.clone(),
            vec![Uniform::new("rect", (x, y, w, h)), Uniform::new("radius", self.radius)],
        );
        let res = RenderElement::<GlesRenderer>::draw(&self.inner, frame, src, dst, damage, opaque_regions, cache);
        frame.clear_tex_program_override();
        res
    }

    fn underlying_storage(&self, _renderer: &mut GlesRenderer) -> Option<UnderlyingStorage<'_>> {
        None // never scan out directly: the corners need the shader
    }
}

// ----------------------------------------------------------------------------------------- wallpaper

pub struct Wallpaper {
    buffer: MemoryRenderBuffer,
    key: String,
}

fn load_wallpaper(path: &str, mode: WallpaperMode, w: u32, h: u32, bg: [f32; 4], scale: i32) -> Option<MemoryRenderBuffer> {
    // `w`×`h` are logical; the image is prepared at the physical (scaled) size.
    let (w, h) = (w * scale as u32, h * scale as u32);
    use image::{imageops, imageops::FilterType, Rgba, RgbaImage};
    let src = image::open(path).map_err(|e| tracing::warn!("wallpaper {path}: {e}")).ok()?.to_rgba8();
    let (iw, ih) = (src.width() as f64, src.height() as f64);
    let (ow, oh) = (w as f64, h as f64);
    let bgpx = Rgba([(bg[0] * 255.0) as u8, (bg[1] * 255.0) as u8, (bg[2] * 255.0) as u8, 255]);
    let mut canvas = RgbaImage::from_pixel(w, h, bgpx);
    let (scaled, x, y) = match mode {
        WallpaperMode::Stretch => (imageops::resize(&src, w, h, FilterType::Triangle), 0i64, 0i64),
        WallpaperMode::Center => (src, (ow - iw) as i64 / 2, (oh - ih) as i64 / 2),
        WallpaperMode::Fill | WallpaperMode::Fit => {
            let k = if mode == WallpaperMode::Fill { (ow / iw).max(oh / ih) } else { (ow / iw).min(oh / ih) };
            let (nw, nh) = (((iw * k).round() as u32).max(1), ((ih * k).round() as u32).max(1));
            (imageops::resize(&src, nw, nh, FilterType::Triangle), (w as i64 - nw as i64) / 2, (h as i64 - nh as i64) / 2)
        }
    };
    imageops::overlay(&mut canvas, &scaled, x, y);
    Some(MemoryRenderBuffer::from_slice(
        canvas.as_raw(),
        Fourcc::Abgr8888,
        (w as i32, h as i32),
        scale,
        Transform::Normal,
        Some(vec![Rectangle::from_size((w as i32, h as i32).into())]),
    ))
}

/// The pointer image: an xcursor theme arrow if one is installed, else a built-in arrow.
pub struct Cursor {
    buffer: MemoryRenderBuffer,
    hotspot: (i32, i32),
}

impl Cursor {
    /// `size` is the logical pointer size; on a 2× screen the image is loaded twice as large.
    pub fn load(size: u32, scale: i32) -> Self {
        let theme = std::env::var("XCURSOR_THEME").unwrap_or_else(|_| "default".into());
        match load_xcursor(&theme, size * scale as u32) {
            Some((rgba, w, h, hx, hy)) => Self::from_rgba(&rgba, w, h, (hx / scale, hy / scale), scale),
            None => {
                let (rgba, w, h) = builtin_arrow();
                Self::from_rgba(&rgba, w, h, (1, 1), 1)
            }
        }
    }

    fn from_rgba(rgba: &[u8], w: u32, h: u32, hotspot: (i32, i32), scale: i32) -> Self {
        let buffer = MemoryRenderBuffer::from_slice(
            rgba,
            Fourcc::Argb8888,
            (w as i32, h as i32),
            scale,
            Transform::Normal,
            None,
        );
        Self { buffer, hotspot }
    }
}

fn load_xcursor(theme: &str, size: u32) -> Option<(Vec<u8>, u32, u32, i32, i32)> {
    let theme = xcursor::CursorTheme::load(theme);
    let path = theme.load_icon("default").or_else(|| theme.load_icon("left_ptr"))?;
    let mut data = Vec::new();
    std::fs::File::open(path).ok()?.read_to_end(&mut data).ok()?;
    let images = xcursor::parser::parse_xcursor(&data)?;
    let img = images.iter().min_by_key(|i| (size as i32 - i.size as i32).abs())?;
    Some((img.pixels_rgba.clone(), img.width, img.height, img.xhot as i32, img.yhot as i32))
}

/// A small white arrow with a black outline, generated at runtime (premultiplied-free ARGB bytes).
fn builtin_arrow() -> (Vec<u8>, u32, u32) {
    const W: usize = 16;
    const H: usize = 24;
    let mut px = vec![0u8; W * H * 4];
    for y in 0..H {
        let width = if y < 16 { y / 1 + 1 } else { 16usize.saturating_sub(y - 8) };
        for x in 0..width.min(W) {
            let edge = x == 0 || x + 1 == width.min(W) || y == 0 || y == H - 1;
            let c = if edge { 0u8 } else { 255u8 };
            let i = (y * W + x) * 4;
            // Argb8888 is BGRA in memory on little-endian.
            px[i..i + 4].copy_from_slice(&[c, c, c, 255]);
        }
    }
    (px, W as u32, H as u32)
}

impl SaforaState {
    /// (Re)load the wallpaper if the file, mode or output size changed.
    fn refresh_wallpaper(&mut self, name: &str, w: u32, h: u32, scale: i32) {
        let d = self.config.desktop.clone();
        if d.wallpaper.is_empty() {
            self.wallpaper.remove(name);
            return;
        }
        let mtime = std::fs::metadata(&d.wallpaper).and_then(|m| m.modified()).ok();
        let key = format!("{}|{:?}|{w}x{h}@{scale}|{:?}|{}", d.wallpaper, d.wallpaper_mode, mtime, self.config.colors.background);
        if self.wallpaper.get(name).is_some_and(|wp| wp.key == key) {
            return;
        }
        let bg = parse_color(&self.config.colors.background);
        match load_wallpaper(&d.wallpaper, d.wallpaper_mode, w, h, bg, scale) {
            Some(buffer) => {
                self.wallpaper.insert(name.to_string(), Wallpaper { buffer, key });
            }
            None => {
                self.wallpaper.remove(name);
            }
        }
    }

    /// Elements front-to-back: pointer, overlay/top layers, borders, windows, bottom layers, wallpaper.
    pub fn build_elements(&mut self, renderer: &mut GlesRenderer, output: &Output) -> Vec<SaforaElement> {
        let mut elements: Vec<SaforaElement> = Vec::new();
        let scale = Scale::from(output.current_scale().fractional_scale());
        let Some(geo) = self.space.output_geometry(output) else { return elements };
        if self.blanked {
            return elements;
        }
        let out_h = geo.size.h as f32;
        let off = geo.loc;
        let is_primary = self.output.as_ref() == Some(output);
        let fullscreen_active = self.clients.iter().any(|c| {
            c.fullscreen && c.workspace == self.active_ws && c.fs_rect.map_or(is_primary, |r| r.x == geo.loc.x && r.y == geo.loc.y)
        });

        // Pointer.
        if geo.to_f64().contains(self.pointer_location) {
            let rel = self.pointer_location - geo.loc.to_f64();
            match &self.cursor_status {
                CursorImageStatus::Hidden => {}
                CursorImageStatus::Surface(surface) => {
                    let hot = with_states(surface, |s| {
                        s.data_map.get::<Mutex<CursorImageAttributes>>().unwrap().lock().unwrap().hotspot
                    });
                    let loc: Point<i32, Physical> = (rel - hot.to_f64()).to_physical(scale).to_i32_round();
                    elements.extend(
                        render_elements_from_surface_tree::<_, WaylandSurfaceRenderElement<GlesRenderer>>(
                            renderer, surface, loc, scale, 1.0, Kind::Cursor,
                        )
                        .into_iter()
                        .map(SaforaElement::from),
                    );
                }
                CursorImageStatus::Named(_) => {
                    let hot: Point<f64, Logical> = (self.cursor.hotspot.0 as f64, self.cursor.hotspot.1 as f64).into();
                    let loc = (rel - hot).to_physical(scale);
                    if let Ok(el) = MemoryRenderBufferRenderElement::from_buffer(
                        renderer, loc, &self.cursor.buffer, None, None, None, Kind::Cursor,
                    ) {
                        elements.push(SaforaElement::from(el));
                    }
                }
            }
        }

        // Layer-shell surfaces (and their popups) above the windows.
        let layer_elements = |renderer: &mut GlesRenderer, layers: &[Layer]| -> Vec<SaforaElement> {
            use smithay::backend::renderer::element::AsRenderElements;
            let map = layer_map_for_output(output);
            let mut out = Vec::new();
            for layer in layers {
                for surf in map.layers_on(*layer).rev() {
                    if let Some(lgeo) = map.layer_geometry(surf) {
                        let loc = lgeo.loc.to_physical_precise_round(scale);
                        out.extend(
                            AsRenderElements::<GlesRenderer>::render_elements::<WaylandSurfaceRenderElement<GlesRenderer>>(
                                surf, renderer, loc, scale, 1.0,
                            )
                            .into_iter()
                            .map(SaforaElement::from),
                        );
                    }
                }
            }
            out
        };
        elements.extend(layer_elements(renderer, &[Layer::Overlay]));
        if self.locked {
            return elements; // nothing but the lock screen (and the pointer) is drawn
        }
        if !fullscreen_active {
            elements.extend(layer_elements(renderer, &[Layer::Top]));
        }

        // Windows (topmost first) with their borders.
        let (active_col, inactive_col) = self.border_colors();
        let bw = self.config.layout.border_width.max(0);
        let radius = self.config.windows.corner_radius.max(0);
        let (op_active, op_inactive) = (self.config.windows.opacity_active as f32, self.config.windows.opacity_inactive as f32);
        let focused = self.focused;
        let flip = self.fb_flip;
        let shaders = self.shaders.clone();

        let windows: Vec<Window> = self.space.elements().rev().cloned().collect();
        for window in windows {
            let Some(idx) = self.client_index_for_surface(window.toplevel().unwrap().wl_surface()) else { continue };
            let Some(wgeo) = self.space.element_geometry(&window) else { continue };
            let Some(wloc) = self.space.element_location(&window) else { continue };
            // Only windows on this monitor; coordinates become relative to it.
            if !wgeo.overlaps(geo) {
                continue;
            }
            let wgeo = Rectangle::<i32, Logical>::new(wgeo.loc - off, wgeo.size);
            let wloc = wloc - off;
            let (is_focused, is_full, id) = {
                let c = &self.clients[idx];
                (Some(c.id) == focused, c.fullscreen, c.id)
            };
            let alpha = if is_full { 1.0 } else if is_focused { op_active } else { op_inactive };
            let r = if is_full { 0 } else { radius };
            let render_loc: Point<i32, Physical> = (wloc - window.geometry().loc).to_physical_precise_round(scale);
            let surface = window.toplevel().unwrap().wl_surface().clone();

            // Popups first (they sit above the window and are never clipped).
            for (popup, offset) in PopupManager::popups_for_surface(&surface) {
                let off = (window.geometry().loc + offset - popup.geometry().loc).to_physical_precise_round(scale);
                elements.extend(
                    render_elements_from_surface_tree::<_, WaylandSurfaceRenderElement<GlesRenderer>>(
                        renderer, popup.wl_surface(), render_loc + off, scale, 1.0, Kind::Unspecified,
                    )
                    .into_iter()
                    .map(SaforaElement::from),
                );
            }

            // Border.
            if bw > 0 && !is_full {
                let color = if is_focused { active_col } else { inactive_col };
                let border_area = Rectangle::<i32, Logical>::new(
                    (wgeo.loc.x - bw, wgeo.loc.y - bw).into(),
                    (wgeo.size.w + 2 * bw, wgeo.size.h + 2 * bw).into(),
                );
                if r > 0 && shaders.is_some() {
                    let sh = shaders.as_ref().unwrap();
                    let uniforms = vec![
                        Uniform::new("color", (color[0] * color[3], color[1] * color[3], color[2] * color[3], color[3])),
                        Uniform::new("radius", r as f32),
                        Uniform::new("bw", bw as f32),
                    ];
                    let c = &mut self.clients[idx];
                    let sig = (border_area, color, r, bw);
                    match c.ring.as_mut() {
                        Some((el, last)) => {
                            if *last != sig {
                                el.resize(border_area, None);
                                el.update_uniforms(uniforms);
                                *last = sig;
                            }
                            elements.push(SaforaElement::from(el.clone()));
                        }
                        None => {
                            let el = PixelShaderElement::new(sh.ring.clone(), border_area, None, 1.0, uniforms, Kind::Unspecified);
                            elements.push(SaforaElement::from(el.clone()));
                            c.ring = Some((el, sig));
                        }
                    }
                } else {
                    let color32 = Color32F::from(color);
                    let (x, y, w, h) = (wgeo.loc.x, wgeo.loc.y, wgeo.size.w, wgeo.size.h);
                    let rects = [
                        ((x - bw, y - bw), (w + 2 * bw, bw)),
                        ((x - bw, y + h), (w + 2 * bw, bw)),
                        ((x - bw, y), (bw, h)),
                        ((x + w, y), (bw, h)),
                    ];
                    let c = &mut self.clients[idx];
                    for (buf, (pos, size)) in c.borders.iter_mut().zip(rects) {
                        buf.update(size, color32);
                        let loc = Point::<i32, Logical>::from(pos).to_physical_precise_round(scale);
                        elements.push(SaforaElement::from(SolidColorRenderElement::from_buffer(
                            buf, loc, scale, 1.0, Kind::Unspecified,
                        )));
                    }
                }
            }
            let _ = id;

            // The window surface itself, clipped to rounded corners when requested.
            let tree = render_elements_from_surface_tree::<_, WaylandSurfaceRenderElement<GlesRenderer>>(
                renderer, &surface, render_loc, scale, alpha, Kind::Unspecified,
            );
            match (r > 0, shaders.as_ref()) {
                (true, Some(sh)) => {
                    let phys: Rectangle<i32, Physical> = wgeo.to_physical_precise_round(scale);
                    let y = if flip { out_h - (phys.loc.y + phys.size.h) as f32 } else { phys.loc.y as f32 };
                    let rect = [phys.loc.x as f32, y, phys.size.w as f32, phys.size.h as f32];
                    for inner in tree {
                        elements.push(SaforaElement::from(RoundedSurface {
                            inner,
                            program: sh.round_tex.clone(),
                            rect,
                            radius: r as f32 * scale.x as f32,
                        }));
                    }
                }
                _ => elements.extend(tree.into_iter().map(SaforaElement::from)),
            }
        }

        // Layers beneath the windows, then the wallpaper.
        elements.extend(layer_elements(renderer, &[Layer::Bottom, Layer::Background]));
        let (pw, ph) = (geo.size.w as u32, geo.size.h as u32);
        let oname = output.name();
        self.refresh_wallpaper(&oname, pw, ph, output.current_scale().integer_scale());
        if let Some(wp) = self.wallpaper.get(&oname) {
            if let Ok(el) = MemoryRenderBufferRenderElement::from_buffer(
                renderer, (0.0, 0.0), &wp.buffer, None, None, None, Kind::Unspecified,
            ) {
                elements.push(SaforaElement::from(el));
            }
        }
        elements
    }

    pub fn clear_color(&self) -> Color32F {
        if self.blanked || self.locked {
            return Color32F::from([0.0, 0.0, 0.0, 1.0]);
        }
        Color32F::from(parse_color(&self.config.colors.background))
    }

    /// Send frame callbacks so clients render their next frame.
    pub fn send_frames(&self, output: &Output) {
        let now = self.start_time.elapsed();
        for c in self.clients.iter().filter(|c| c.workspace == self.active_ws) {
            c.window.send_frame(output, now, Some(Duration::ZERO), |_, _| Some(output.clone()));
        }
        for layer in layer_map_for_output(output).layers() {
            layer.send_frame(output, now, Some(Duration::ZERO), |_, _| Some(output.clone()));
        }
        if let CursorImageStatus::Surface(surface) = &self.cursor_status {
            send_frames_surface_tree(surface, output, now, Some(Duration::ZERO), |_, _| Some(output.clone()));
        }
    }

    /// Per-iteration housekeeping shared by both backends.
    pub fn after_dispatch(&mut self) {
        self.space.refresh();
        self.popups.cleanup();
        for output in self.space.outputs().cloned().collect::<Vec<_>>() {
            layer_map_for_output(&output).cleanup();
        }
        self.broadcast_state();
        let _ = self.display_handle.flush_clients();
    }
}
