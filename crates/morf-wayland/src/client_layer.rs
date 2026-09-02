use morf_region::Region;

use smithay_client_toolkit::compositor::FrameCallbackData;
use smithay_client_toolkit::shell::WaylandSurface;
use smithay_client_toolkit::shell::wlr_layer::{
    Anchor, KeyboardInteractivity as WlrKeyboardInteractivity, Layer,
};
use smithay_client_toolkit::shell::xdg::window::WindowDecorations;
use wayland_client::protocol::{wl_output, wl_surface};

use crate::{state_types::*, surface_types::*, types::*};

/// Identifier of the layer surface every client creates first.
///
/// The role is plural, but one surface is still the shell's own: it is the one
/// `connect` opens, the one whose size and scale the bare accessors report, and
/// the parent an unqualified popup attaches to.
pub const PRIMARY_LAYER: u64 = 0;

/// Converts configured anchor edges into the layer-shell bitmask.
///
/// `open_layer` and `set_layer_geometry` issue `set_anchor` on the same object
/// with the same meaning, so they share one conversion: two copies would let a
/// runtime update silently re-anchor a surface the creation path had pinned
/// somewhere else.
pub(crate) fn layer_anchor_mask(anchors: LayerAnchors) -> Anchor {
    let mut mask = Anchor::empty();
    if anchors.top {
        mask |= Anchor::TOP;
    }
    if anchors.right {
        mask |= Anchor::RIGHT;
    }
    if anchors.bottom {
        mask |= Anchor::BOTTOM;
    }
    if anchors.left {
        mask |= Anchor::LEFT;
    }
    mask
}

/// Converts a keyboard focus policy into its layer-shell interactivity.
pub(crate) fn layer_interactivity(focus: KeyboardFocus) -> WlrKeyboardInteractivity {
    match focus {
        KeyboardFocus::None => WlrKeyboardInteractivity::None,
        KeyboardFocus::Exclusive => WlrKeyboardInteractivity::Exclusive,
        KeyboardFocus::OnDemand => WlrKeyboardInteractivity::OnDemand,
    }
}

impl LayerClient {
    /// Resolves a configured output name against the compositor's current set.
    pub(crate) fn layer_output(
        &self,
        name: Option<&str>,
    ) -> Result<Option<wl_output::WlOutput>, WaylandError> {
        let Some(name) = name else {
            return Ok(None);
        };
        self.state
            .outputs
            .outputs()
            .find(|output| {
                self.state
                    .outputs
                    .info(output)
                    .and_then(|info| info.name)
                    .as_deref()
                    == Some(name)
            })
            .map(Some)
            .ok_or_else(|| WaylandError(format!("Wayland output `{name}` is unavailable")))
    }

    /// Creates or replaces one wlr-layer-shell surface under a client-local id.
    pub fn open_layer(&mut self, id: u64, config: BarConfig) -> Result<(), WaylandError> {
        self.close_layer(id);
        let qh = self.queue.handle();
        let output = self.layer_output(config.output.as_deref())?;
        let surface = self.state.compositor.create_surface(&qh);
        surface.set_buffer_scale(1);
        let layer = match &self.state.layer_shell {
            Some(shell) => {
                let layer = shell.create_layer_surface(
                    &qh,
                    surface,
                    match config.layer {
                        ShellLayer::Background => Layer::Background,
                        ShellLayer::Bottom => Layer::Bottom,
                        ShellLayer::Top => Layer::Top,
                        ShellLayer::Overlay => Layer::Overlay,
                    },
                    Some(config.namespace.clone()),
                    output.as_ref(),
                );
                layer.set_anchor(layer_anchor_mask(config.anchors));
                layer.set_keyboard_interactivity(layer_interactivity(config.keyboard_focus));
                layer.set_size(config.width, config.height);
                layer.set_margin(
                    config.margin_top,
                    config.margin_right,
                    config.margin_bottom,
                    config.margin_left,
                );
                layer.set_exclusive_zone(config.exclusive_zone);
                ShellSurface::Layer(layer)
            }
            // No layer-shell: stand the surface up as a fullscreen toplevel
            // instead. Anchors, margins and the exclusive zone are dropped
            // rather than approximated, because a toplevel has no way to
            // express them and inventing an offset would put the surface
            // somewhere the configuration never asked for. Fullscreen is asked
            // for explicitly instead of assumed: a compositor that honours it
            // gives the whole output, which is what a shell surface covers, and
            // one that refuses still maps the window at its requested size.
            None => {
                let window =
                    self.state
                        .xdg_shell
                        .create_window(surface, WindowDecorations::None, &qh);
                window.set_title(config.namespace.clone());
                window.set_app_id(config.namespace.clone());
                window.set_fullscreen(None);
                ShellSurface::Window(Box::new(window))
            }
        };
        let fractional_scale = self
            .state
            .fractional_manager
            .as_ref()
            .map(|manager| manager.get_fractional_scale(layer.wl_surface(), &qh, id));
        let viewport = self
            .state
            .viewporter
            .as_ref()
            .map(|manager| manager.get_viewport(layer.wl_surface(), &qh, ()));
        // One background-effect object for the life of the surface. Asking a
        // second time is a protocol error, and the object is only a handle to
        // call `set_blur_region` on — so it is made here, once, and the paint
        // path only ever uses it. A surface that never asks for a blur has paid
        // for one small object it does not use, which is cheaper than the
        // interior mutability that creating it lazily would need.
        let backdrop = self
            .state
            .background_effect
            .as_ref()
            .map(|manager| manager.get_background_effect(layer.wl_surface(), &qh, ()));
        // A surface with no input region set accepts the pointer over the whole
        // of itself. That is the wrong default for a shell: between this commit
        // and the first paint — which is where the real region is derived from
        // live interactive geometry — the surface would silently swallow every
        // click over its own area, and a fullscreen overlay would swallow the
        // desktop. So it starts claiming nothing and the first paint opens up
        // whatever the configuration actually asked for.
        let empty = self.state.compositor.wl_compositor().create_region(&qh, ());
        layer.wl_surface().set_input_region(Some(&empty));
        empty.destroy();
        layer.commit();
        self.state.layers.insert(
            id,
            LayerRecord {
                surface: layer,
                backdrop,
                fractional_scale,
                viewport,
                width: config.width.max(1),
                height: config.height.max(1),
                scale_120: 120,
                wants_blank: false,
                configured: false,
                blank: None,
            },
        );
        self.connection
            .flush()
            .map_err(|error| WaylandError(format!("Wayland flush failed: {error}")))
    }

    /// Re-issues the geometry of a layer surface that is already open.
    ///
    /// wlr-layer-shell permits size, anchors, margins, exclusive zone and
    /// keyboard interactivity to change on a mapped surface; namespace, layer
    /// and output do not, and stay the business of [`LayerClient::open_layer`].
    /// Nothing here destroys an object, so the zwlr surface, the wl_surface, the
    /// fractional scale, the viewport and whatever renders into them all
    /// survive: the compositor answers with a configure, and the surface
    /// resizes in place instead of unmapping and coming back.
    pub fn set_layer_geometry(&self, id: u64, config: &BarConfig) -> Result<(), WaylandError> {
        let record = self
            .state
            .layers
            .get(&id)
            .ok_or_else(|| WaylandError("layer surface is not open".into()))?;
        // Nothing to re-anchor on a toplevel: it has no anchors, margins or
        // exclusive zone to set, and the compositor sizes it.
        let Some(layer) = record.surface.as_layer() else {
            return Ok(());
        };
        layer.set_size(config.width, config.height);
        layer.set_anchor(layer_anchor_mask(config.anchors));
        layer.set_margin(
            config.margin_top,
            config.margin_right,
            config.margin_bottom,
            config.margin_left,
        );
        layer.set_exclusive_zone(config.exclusive_zone);
        layer.set_keyboard_interactivity(layer_interactivity(config.keyboard_focus));
        layer.commit();
        self.connection
            .flush()
            .map_err(|error| WaylandError(format!("Wayland flush failed: {error}")))
    }

    /// Asks a layer surface to map itself with a single transparent pixel.
    ///
    /// A surface that never attaches a buffer stays unmapped, and a compositor
    /// derives an output's usable area only from the layer surfaces it actually
    /// arranges — so an unmapped reserver reserves nothing at all. The protocol
    /// requires the first commit to carry no buffer and the configure that
    /// follows to be acknowledged before one may be attached, so this records
    /// the intent and the configure handler completes it.
    pub fn map_layer_blank(&mut self, id: u64) -> Result<(), WaylandError> {
        let Some(record) = self.state.layers.get_mut(&id) else {
            return Ok(());
        };
        if record.blank.is_some() {
            return Ok(());
        }
        record.wants_blank = true;
        self.state.attach_blank_buffer(id);
        self.connection
            .flush()
            .map_err(|error| WaylandError(format!("Wayland flush failed: {error}")))
    }

    /// Destroys one layer surface when it is open.
    pub fn close_layer(&mut self, id: u64) {
        if self.state.layers.remove(&id).is_none() {
            return;
        }
        self.forget_surface(SurfaceRole::Layer(id));
    }

    /// Drops the input state pointing at a surface that has gone away.
    ///
    /// Every close path needs this and only one of the three did it, so a
    /// finger still down on a popup as it closed left a touch point addressed
    /// to a surface that no longer existed — and the next motion for that
    /// finger was delivered against it.
    pub(crate) fn forget_surface(&mut self, role: SurfaceRole) {
        if self.state.keyboard_surface == Some(role) {
            self.state.keyboard_surface = None;
        }
        self.state
            .touch_points
            .retain(|_, (_, holder)| *holder != role);
    }

    /// Returns the wl_surface backing one layer surface.
    pub fn layer_surface(&self, id: u64) -> Option<&wl_surface::WlSurface> {
        self.state
            .layers
            .get(&id)
            .map(|layer| layer.surface.wl_surface())
    }

    /// Returns the configured logical dimensions of one layer surface.
    pub fn layer_logical_size(&self, id: u64) -> Option<(u32, u32)> {
        self.state
            .layers
            .get(&id)
            .map(|layer| (layer.width, layer.height))
    }

    /// Returns the preferred scale of one layer surface in 120ths.
    pub fn layer_scale_120(&self, id: u64) -> Option<u32> {
        self.state.layers.get(&id).map(|layer| layer.scale_120)
    }

    /// Requests a compositor callback for one layer surface's next frame.
    pub fn request_layer_frame(&self, id: u64) {
        let Some(surface) = self.layer_surface(id) else {
            return;
        };
        let qh = self.queue.handle();
        surface.frame(&qh, FrameCallbackData(surface.clone()));
    }

    /// Commits pending state on one layer surface without attaching a buffer.
    pub fn commit_layer(&self, id: u64) {
        if let Some(surface) = self.layer_surface(id) {
            surface.commit();
        }
    }

    /// Returns an owned raw-window target for one layer surface.
    pub fn layer_window_target(&self, id: u64) -> Option<WaylandWindowTarget> {
        self.layer_surface(id).map(|surface| WaylandWindowTarget {
            backend: self.connection.backend(),
            surface: surface.clone(),
        })
    }

    /// Applies the default, empty, or rectangular input region to one surface.
    pub fn set_layer_input_region(&self, id: u64, rectangles: Option<&[InputRect]>) {
        let Some(surface) = self.layer_surface(id) else {
            return;
        };
        let Some(rectangles) = rectangles else {
            surface.set_input_region(None);
            return;
        };
        let qh = self.queue.handle();
        let region = self.state.compositor.wl_compositor().create_region(&qh, ());
        for rectangle in rectangles {
            if rectangle.width > 0 && rectangle.height > 0 {
                region.add(rectangle.x, rectangle.y, rectangle.width, rectangle.height);
            }
        }
        surface.set_input_region(Some(&region));
        region.destroy();
    }

    /// Builds and applies a composable logical input region to one surface.
    pub fn set_layer_composed_input_region(
        &self,
        id: u64,
        regions: &[Region],
    ) -> Result<(), WaylandError> {
        let (width, height) = self
            .layer_logical_size(id)
            .ok_or_else(|| WaylandError("layer surface is not open".into()))?;
        let rectangles = morf_region::build(width, height, regions)
            .map_err(|error| WaylandError(error.to_string()))?;
        self.set_layer_input_region(id, Some(&rectangles));
        Ok(())
    }
}
