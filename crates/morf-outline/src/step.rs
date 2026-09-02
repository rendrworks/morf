/// One piece of an outline, as every source of outlines describes it.
///
/// A font hands back move/line/quadratic/cubic/close and so does an SVG, which
/// is not a coincidence: both are describing the same thing, and this is that
/// thing with the provenance taken off. Converting into it is the whole of what
/// a new source of outlines has to do.
///
/// Coordinates are in whatever space the source used. Nothing here rescales
/// them — the field does that when it places the outline in its own box.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Step {
    Move(f32, f32),
    Line(f32, f32),
    /// One control point, then the end.
    Quad(f32, f32, f32, f32),
    /// Two control points, then the end.
    Cubic(f32, f32, f32, f32, f32, f32),
    Close,
}
