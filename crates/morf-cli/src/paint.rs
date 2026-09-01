use morf_layout::{Layout, Size};
use morf_lua::{LayerSurfaceConfig, Runtime};
use morf_region::{Rect as RegionRect, Region};
use morf_render::{RenderEngine, WgpuBackend};
use morf_scene::NodeHandle;
use morf_wayland::{InputRect, LayerClient, PRIMARY_LAYER, physical_size};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use crate::{surface_layers::*, surfaces::*};

pub(crate) fn paint(
    runtime: &Runtime,
    renderer: &mut RenderEngine<WgpuBackend>,
    client: &LayerClient,
    root: NodeHandle,
    cache: Option<&CachedLayout>,
) -> Result<CachedLayout, String> {
    paint_layer(
        runtime,
        renderer,
        client,
        PRIMARY_LAYER,
        root,
        &runtime.layer_surface_config(),
        cache,
    )
}

/// Lays out, masks, and renders the scene subtree of one layer surface.
///
/// Every layer surface derives its input region the same way: an explicit mask
/// when the configuration supplies one, and otherwise the live geometry of the
/// interactive items, recomputed here so it tracks the surface as it changes.
/// A layout, and what it was computed from.
///
/// Layout is the most expensive thing a frame does, and most frames change
/// nothing it reads — a colour easing, a morph advancing, an opacity fading.
/// Keeping what the last one was built from lets those frames reuse it.
#[derive(Clone)]
pub(crate) struct CachedLayout {
    pub(crate) layout: Layout,
    pub(crate) revision: u64,
    pub(crate) size: (u32, u32),
    pub(crate) scale_120: u32,
    /// The input region last handed to the compositor for this surface.
    ///
    /// It is double-buffered surface state, so it persists until it is set
    /// again — sending an identical one costs a region object, a round of
    /// protocol traffic and the derivation that produced it, and changes
    /// nothing.
    pub(crate) input: Vec<InputRect>,
    /// The backdrop-blur shapes last handed to the compositor.
    ///
    /// The *shapes*, not the rectangles they rasterise to, because rasterising
    /// is the expensive half — six milliseconds for a full-screen region in
    /// release — and comparing first means a swarm that has drifted less than
    /// the grid does no work at all rather than doing all of it and discovering
    /// the answer was the same.
    pub(crate) backdrop: Vec<Region>,
}

impl CachedLayout {
    /// Whether this layout still describes the scene.
    ///
    /// Everything `Layout::compute` reads is either a property layout depends
    /// on — which moves the revision — or one of the two inputs handed to it.
    /// A surface that has resized, or that the compositor now presents at a
    /// different scale, has to be laid out again however still the scene is.
    pub(crate) fn still_valid(&self, revision: u64, size: (u32, u32), scale_120: u32) -> bool {
        self.revision == revision && self.size == size && self.scale_120 == scale_120
    }
}

impl std::ops::Deref for CachedLayout {
    type Target = Layout;

    fn deref(&self) -> &Layout {
        &self.layout
    }
}

pub(crate) fn paint_layer(
    runtime: &Runtime,
    renderer: &mut RenderEngine<WgpuBackend>,
    client: &LayerClient,
    layer: u64,
    root: NodeHandle,
    config: &LayerSurfaceConfig,
    cache: Option<&CachedLayout>,
) -> Result<CachedLayout, String> {
    let scene = runtime.scene();
    let (width, height) = client
        .layer_logical_size(layer)
        .ok_or_else(|| "layer surface disappeared while painting".to_owned())?;
    let scale_120 = client.layer_scale_120(layer).unwrap_or(120);
    let revision = scene.layout_revision();
    let reusable = cache.filter(|cached| cached.still_valid(revision, (width, height), scale_120));
    let layout = match reusable {
        Some(cached) => cached.layout.clone(),
        None => Layout::compute(
            &scene,
            root,
            Size {
                width: width as f64,
                height: height as f64,
            },
            renderer.backend_mut(),
        )
        .map_err(|error| error.to_string())?,
    };
    let input = if let Some(regions) = &config.input_regions {
        // A configured mask is a static surface setting — nothing animates it —
        // so rasterising it and re-sending it every paint asks the compositor
        // to rebuild an identical region sixty times a second. The branch below
        // has always deduped; this one opted out of the cache by returning an
        // empty vector, which also made every frame look like a change.
        //
        // The sentinel is what the cache compares: an empty vector would match
        // a surface that genuinely has no interactive area, so a shape that
        // stands for "the configured mask, unchanged" is stored instead.
        let input = vec![MASK_SENTINEL];
        if cache.is_none_or(|cached| cached.input != input) {
            client
                .set_layer_composed_input_region(layer, regions)
                .map_err(|error| error.to_string())?;
        }
        input
    } else {
        let input = layout
            .input_geometry(&scene)
            .map_err(|error| error.to_string())?
            .into_iter()
            .map(|geometry| {
                let left = geometry.x.floor() as i32;
                let top = geometry.y.floor() as i32;
                let right = (geometry.x + geometry.width).ceil() as i32;
                let bottom = (geometry.y + geometry.height).ceil() as i32;
                InputRect {
                    x: left,
                    y: top,
                    width: right - left,
                    height: bottom - top,
                }
            })
            .collect::<Vec<_>>();
        if cache.is_none_or(|cached| cached.input != input) {
            client.set_layer_input_region(layer, Some(&input));
        }
        input
    };
    let mut backdrop = Vec::new();
    // Where the compositor should blur what is behind this surface. Nothing is
    // read back: the blur happens on the far side of this call, underneath a
    // surface that is about to be composited over it, and the only thing that
    // makes it visible is the alpha this configuration painted with.
    if client.supports_backdrop_blur() {
        let shapes: Vec<Region> = layout
            .backdrop_geometry(&scene)
            .map_err(|error| error.to_string())?
            .into_iter()
            .map(|(geometry, radii)| Region {
                // Whole pixels, not grid cells. Quantising the *position*
                // here was worth nothing — a moving shape never compares equal
                // to its cached self whatever the grid, and a still one
                // compares equal without any — and it cost up to half a cell of
                // registration against the shape drawn over it, in a direction
                // that changed every frame.
                rect: RegionRect {
                    x: geometry.x.floor() as i32,
                    y: geometry.y.floor() as i32,
                    width: (geometry.width.ceil() as i32).max(0),
                    height: (geometry.height.ceil() as i32).max(0),
                },
                shape: morf_region::Shape::Box,
                params: morf_region::ShapeParams {
                    radii,
                    ..morf_region::ShapeParams::default()
                },
                ..Region::default()
            })
            .collect();
        if cache.is_none_or(|cached| cached.backdrop != shapes) {
            let rectangles = if shapes.is_empty() {
                Vec::new()
            } else {
                morf_region::build_scaled(width, height, &shapes, morf_region::COVERED_EDGE_GRID)
                    .map_err(|error| error.to_string())?
            };
            client
                .set_layer_backdrop_region(layer, Some(&rectangles))
                .map_err(|error| error.to_string())?;
        }
        backdrop = shapes;
    }

    client.request_layer_frame(layer);
    let surface = client
        .layer_surface(layer)
        .ok_or_else(|| "layer surface disappeared while painting".to_owned())?;
    let damage = renderer
        .render(&scene, &layout, scale_120, |damage| {
            // What actually changed, rather than the whole surface. A
            // compositor recomposites the area a client declares, so a
            // fullscreen overlay that declares everything costs a full screen
            // of blending every frame however little of it moved.
            for rect in damage {
                surface.damage_buffer(
                    rect.x as i32,
                    rect.y as i32,
                    rect.width as i32,
                    rect.height as i32,
                );
            }
        })
        .map_err(|error| error.to_string())?;
    if damage.is_empty() {
        client.commit_layer(layer);
    }
    drop(scene);
    runtime.observe_layout(&layout);
    Ok(CachedLayout {
        layout,
        revision,
        size: (width, height),
        scale_120,
        input,
        backdrop,
    })
}

/// Paints one configured layer surface into its own renderer.
pub(crate) fn paint_layer_surface(
    runtime: &Runtime,
    client: &LayerClient,
    surface: &mut AuxiliarySurface,
) -> Result<(), String> {
    // Cleared here rather than at one of the two call sites, because there are
    // two: the frame callback honoured the flag and the main repaint block did
    // not, so an animating configured layer surface was painted twice for every
    // tick — once by each — and the flag it was supposed to be gated on was
    // never cleared by the one that ignored it.
    surface.needs_paint = false;
    let Some(renderer) = &mut surface.renderer else {
        return Ok(());
    };
    let config = surface
        .layer_config
        .clone()
        .ok_or_else(|| "layer surface lost its configuration".to_owned())?;
    let painted = paint_layer(
        runtime,
        renderer,
        client,
        window_layer_id(surface.id),
        surface.root,
        &config,
        surface.layout.as_ref(),
    )?;
    surface.layout = Some(painted);
    Ok(())
}

/// Stands in the input cache for "the configured mask, already sent".
///
/// A configured mask is not built from layout, so there is no rectangle list to
/// compare; what the cache needs is only something that is equal to itself and
/// unequal to any real region. The negative extent cannot arise from geometry,
/// which is floored and ceiled from a non-negative rectangle.
pub(crate) const MASK_SENTINEL: InputRect = InputRect {
    x: i32::MIN,
    y: i32::MIN,
    width: -1,
    height: -1,
};

/// Which kind of auxiliary surface a paint is for.
///
/// The popup and the floating window are painted by identical code — they were
/// two copies of the same fifty lines, differing in four identifiers, which is
/// two places for every future fix to have to be applied. The only thing that
/// actually differs is which of the client's four accessors to reach for.
#[derive(Clone, Copy)]
pub(crate) enum AuxiliaryKind {
    Popup,
    Floating,
}

impl AuxiliaryKind {
    pub(crate) fn name(self) -> &'static str {
        match self {
            Self::Popup => "popup",
            Self::Floating => "floating",
        }
    }

    pub(crate) fn request_frame(self, client: &LayerClient, id: u64) {
        match self {
            Self::Popup => client.request_popup_frame(id),
            Self::Floating => client.request_floating_frame(id),
        }
    }

    /// Declares the whole surface damaged, ahead of the render that fills it.
    pub(crate) fn damage(
        self,
        client: &LayerClient,
        id: u64,
        width: u32,
        height: u32,
    ) -> Result<(), String> {
        let surface = match self {
            Self::Popup => client.popup_surface(id),
            Self::Floating => client.floating_surface(id),
        }
        .ok_or_else(|| format!("{} surface disappeared while painting", self.name()))?;
        surface.damage_buffer(0, 0, width as i32, height as i32);
        Ok(())
    }

    pub(crate) fn commit(self, client: &LayerClient, id: u64) {
        let surface = match self {
            Self::Popup => client.popup_surface(id),
            Self::Floating => client.floating_surface(id),
        };
        if let Some(surface) = surface {
            surface.commit();
        }
    }
}

pub(crate) fn paint_popup_surface(
    runtime: &Runtime,
    client: &LayerClient,
    surface: &mut AuxiliarySurface,
) -> Result<(), String> {
    paint_auxiliary_surface(AuxiliaryKind::Popup, runtime, client, surface)
}

pub(crate) fn paint_floating_surface(
    runtime: &Runtime,
    client: &LayerClient,
    surface: &mut AuxiliarySurface,
) -> Result<(), String> {
    paint_auxiliary_surface(AuxiliaryKind::Floating, runtime, client, surface)
}

/// Paints one popup or floating surface.
pub(crate) fn paint_auxiliary_surface(
    kind: AuxiliaryKind,
    runtime: &Runtime,
    client: &LayerClient,
    surface: &mut AuxiliarySurface,
) -> Result<(), String> {
    let Some(renderer) = &mut surface.renderer else {
        return Ok(());
    };
    let scene = runtime.scene();
    let revision = scene.layout_revision();
    let size = (surface.width, surface.height);
    let scale_120 = client.scale_120();
    let reusable = surface.layout.as_ref().filter(|cached| {
        cached.revision == revision && cached.size == size && cached.scale_120 == scale_120
    });
    let layout = match reusable {
        Some(cached) => cached.layout.clone(),
        None => Layout::compute(
            &scene,
            surface.root,
            Size {
                width: surface.width as f64,
                height: surface.height as f64,
            },
            renderer.backend_mut(),
        )
        .map_err(|error| error.to_string())?,
    };
    let (width, height) = physical_size((surface.width, surface.height), client.scale_120());
    kind.damage(client, surface.id, width, height)?;
    let damage = renderer
        .render(&scene, &layout, client.scale_120(), |_| {})
        .map_err(|error| error.to_string())?;
    // Only ask for another callback when this paint actually drew something.
    //
    // Asking unconditionally is a loop with no exit: the callback repaints, the
    // repaint asks for a callback, and a popup that has been sitting still for
    // an hour still costs a full paint every refresh. Anything that changes the
    // scene repaints these surfaces through the main loop anyway, so the
    // callback is a throttle for motion, not the thing that keeps them alive.
    if damage.is_empty() {
        kind.commit(client, surface.id);
    } else {
        kind.request_frame(client, surface.id);
    }
    drop(scene);
    runtime.observe_layout(&layout);
    surface.layout = Some(CachedLayout {
        layout,
        revision,
        size,
        scale_120,
        input: Vec::new(),
        backdrop: Vec::new(),
    });
    Ok(())
}

pub(crate) fn clock_text() -> String {
    jiff::Zoned::now().strftime("%H:%M:%S").to_string()
}

pub(crate) fn until_next_second() -> Duration {
    let elapsed = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default();
    Duration::from_nanos(1_000_000_000 - elapsed.subsec_nanos() as u64)
}
