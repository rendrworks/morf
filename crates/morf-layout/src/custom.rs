//! Containers whose layout is a function the configuration wrote.
//!
//! Two questions, asked once each per pass: how big is this container
//! given what its children want, and where does each child go inside the
//! box it was given. The children's own sizes are measured before either
//! is asked, once, which is what keeps a deep tree linear -- a container
//! cannot ask a child to measure itself again at another width.

use morf_scene::NodeHandle;

use crate::geometry::{Geometry, Size};

/// Who answers for a `Custom` container.
pub trait CustomLayout {
    /// The container's own size, given the room on offer (infinite when
    /// unconstrained) and each child's requested size, in tree order.
    fn measure(
        &mut self,
        node: NodeHandle,
        available: Size,
        children: &[Size],
    ) -> Result<Size, String>;

    /// Where each child goes inside `bounds`, in tree order, as a rectangle
    /// relative to the container. A child left out keeps its requested size
    /// at the origin.
    fn place(
        &mut self,
        node: NodeHandle,
        bounds: Size,
        children: &[Size],
    ) -> Result<Vec<Geometry>, String>;
}

/// The host used when nobody can answer: a `Custom` container is then an
/// error, not a silent stack of children at the origin.
pub struct NoCustom;

impl CustomLayout for NoCustom {
    fn measure(&mut self, _: NodeHandle, _: Size, _: &[Size]) -> Result<Size, String> {
        Err("a custom layout needs a host to run its functions".to_owned())
    }

    fn place(&mut self, _: NodeHandle, _: Size, _: &[Size]) -> Result<Vec<Geometry>, String> {
        Err("a custom layout needs a host to run its functions".to_owned())
    }
}
