//! Asking the compositor to blur what is behind a surface.
//!
//! `ext-background-effect-v1`, the staging protocol that generalised KDE's
//! `org_kde_kwin_blur`. A client never receives the pixels behind it — Wayland
//! does not offer them, and this protocol does not either. What it offers is a
//! region: the compositor blurs its own already-composited result inside that
//! region, and only then blends this surface over the top.
//!
//! Which means the alpha is what reveals it. A panel painted opaque sits on a
//! blurred backdrop nobody can see; a panel painted at a fifth of full alpha is
//! frosted glass. Everything that makes it look like glass rather than like a
//! blur filter — the tint, the grain, the lit edge — is painted here, over a
//! backdrop this process never touches.

use morf_region::Region;

use crate::WaylandError;
use crate::surface_types::LayerClient;

impl LayerClient {
    /// Whether the compositor will blur behind a surface.
    ///
    /// False both when the protocol is absent and when it is present but has
    /// withdrawn the capability, because a configuration cannot act on the
    /// difference: either way it should paint something that stands on its own.
    pub fn supports_backdrop_blur(&self) -> bool {
        self.state.background_effect.is_some() && self.state.blur_capable
    }

    /// Blurs the backdrop inside `rectangles`, in surface-local coordinates.
    ///
    /// An empty slice blurs nothing, which is different from `None`: `None`
    /// clears the effect entirely and lets the surface go back to being an
    /// ordinary one.
    pub fn set_layer_backdrop_region(
        &self,
        id: u64,
        rectangles: Option<&[morf_region::Rect]>,
    ) -> Result<(), WaylandError> {
        if !self.supports_backdrop_blur() {
            return Ok(());
        }
        let Some(backdrop) = self
            .state
            .layers
            .get(&id)
            .ok_or_else(|| WaylandError("layer surface is not open".into()))?
            .backdrop
            .as_ref()
        else {
            return Ok(());
        };
        let qh = self.queue.handle();

        let Some(rectangles) = rectangles else {
            // A null region is the protocol's own way of saying "no effect",
            // and it leaves the object in place to be used again.
            backdrop.set_blur_region(None);
            return Ok(());
        };

        let region = self.state.compositor.wl_compositor().create_region(&qh, ());
        for rectangle in rectangles {
            if rectangle.width > 0 && rectangle.height > 0 {
                region.add(rectangle.x, rectangle.y, rectangle.width, rectangle.height);
            }
        }
        backdrop.set_blur_region(Some(&region));
        region.destroy();
        Ok(())
    }

    /// Builds a blur region from composable shapes and applies it.
    ///
    /// A region is a set of rectangles, but at pixel granularity — so a circle
    /// is not approximated as a shape, it is one span per scanline, and the
    /// merged silhouette of a distance field comes out exactly. What a region
    /// cannot carry is a soft edge: membership is one bit per pixel, so the
    /// boundary of the blur is hard, and it is the surface's own antialiased
    /// painting on top that hides the step.
    pub fn set_layer_composed_backdrop_region(
        &self,
        id: u64,
        regions: &[Region],
    ) -> Result<(), WaylandError> {
        if !self.supports_backdrop_blur() {
            return Ok(());
        }
        let (width, height) = self
            .layer_logical_size(id)
            .ok_or_else(|| WaylandError("layer surface is not open".into()))?;
        let rectangles = morf_region::build(width, height, regions)
            .map_err(|error| WaylandError(error.to_string()))?;
        self.set_layer_backdrop_region(id, Some(&rectangles))
    }
}
