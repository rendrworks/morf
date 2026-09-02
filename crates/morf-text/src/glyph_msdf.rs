// Corners, kept.
//
// A field that stores one distance keeps only the nearest edge, and the contour
// it describes is the set of points at a fixed distance from *something*. Round
// off is not a defect of that — it is what the shape means. Two edges meeting at
// a corner leave a wedge that is equidistant from both, and reading a single
// distance through it draws an arc. Every junction in a `#` comes back filleted,
// and the bigger the letter the wider the fillet.
//
// The way out is to stop asking one number to describe two edges. Each edge is
// assigned two of three channels, so that edges meeting at a corner never share
// all of theirs; each channel records the distance to its own edges only; and
// the shape is read back as the *median* of the three. Away from a corner all
// three agree and the median is the distance. At one, two channels see one edge
// and the third sees the other, and the median follows whichever pair holds —
// which is the corner itself, exact, with no arc in it.

use crate::glyph_fields::Segment;

/// Which of the three channels an edge is written into.
///
/// Two of three, never one and never all: an edge needs two so that a corner
/// can break one of them and leave the other agreeing, and at most two so that
/// something is left to disagree with.
#[derive(Clone, Copy, PartialEq)]
pub(crate) struct Channels(u8);

impl Channels {
    const YELLOW: Self = Self(0b011);
    const CYAN: Self = Self(0b110);
    const MAGENTA: Self = Self(0b101);
    /// Every channel, for a contour with no corners to break.
    pub(crate) const WHITE: Self = Self(0b111);

    fn holds(self, channel: usize) -> bool {
        self.0 & (1 << channel) != 0
    }

    /// The next colour in the cycle, which shares exactly one channel with this
    /// one — so a corner breaks one channel and keeps the other.
    fn next(self) -> Self {
        match self {
            Self::YELLOW => Self::CYAN,
            Self::CYAN => Self::MAGENTA,
            _ => Self::YELLOW,
        }
    }
}

/// One straight piece of an outline, and the channels it is written into.
pub(crate) struct Coloured {
    pub(crate) segment: Segment,
    pub(crate) channels: Channels,
}

/// How sharply two pieces have to turn to count as a corner.
///
/// A flattened curve turns a little at every join and none of those are corners.
/// Three degrees is well above what the flattening tolerance produces and well
/// below the shallowest corner a letter has.
const CORNER_COSINE: f32 = 0.9986;

fn direction(segment: &Segment) -> (f32, f32) {
    let (dx, dy) = (segment.x1 - segment.x0, segment.y1 - segment.y0);
    let length = (dx * dx + dy * dy).sqrt().max(f32::EPSILON);
    (dx / length, dy / length)
}

/// Assigns channels to one closed contour's pieces.
///
/// Colour changes at corners and nowhere else, so a run of pieces along a smooth
/// curve shares a colour and reads as one edge. A contour with fewer than two
/// corners has nothing to break — a circle has no corner to keep — so all three
/// channels carry it and the median is the plain distance.
pub(crate) fn colour_contour(segments: Vec<Segment>) -> Vec<Coloured> {
    let count = segments.len();
    if count < 3 {
        return segments
            .into_iter()
            .map(|segment| Coloured {
                segment,
                channels: Channels::WHITE,
            })
            .collect();
    }

    let mut corner_at = vec![false; count];
    let mut corners = 0;
    for index in 0..count {
        let previous = direction(&segments[(index + count - 1) % count]);
        let here = direction(&segments[index]);
        if previous.0 * here.0 + previous.1 * here.1 < CORNER_COSINE {
            corner_at[index] = true;
            corners += 1;
        }
    }
    if corners < 2 {
        return segments
            .into_iter()
            .map(|segment| Coloured {
                segment,
                channels: Channels::WHITE,
            })
            .collect();
    }

    // Start on a corner, so the first run is a whole edge rather than the tail
    // of one that began before the walk did.
    let start = corner_at.iter().position(|corner| *corner).unwrap_or(0);
    let mut coloured = Vec::with_capacity(count);
    coloured.resize_with(count, || Coloured {
        segment: Segment {
            x0: 0.0,
            y0: 0.0,
            x1: 0.0,
            y1: 0.0,
        },
        channels: Channels::WHITE,
    });
    let mut channels = Channels::YELLOW;
    for step in 0..count {
        let index = (start + step) % count;
        if step > 0 && corner_at[index] {
            channels = channels.next();
        }
        coloured[index] = Coloured {
            segment: Segment {
                x0: segments[index].x0,
                y0: segments[index].y0,
                x1: segments[index].x1,
                y1: segments[index].y1,
            },
            channels,
        };
    }
    coloured
}

/// The nearest distance in each channel, and overall, to a point.
///
/// The sign is the winding number's, taken over every piece regardless of
/// colour: which side of the outline a point is on is a property of the outline,
/// not of how its edges were shared out.
pub(crate) fn distances(coloured: &[Coloured], x: f32, y: f32) -> ([f32; 3], f32) {
    let mut nearest = [f32::MAX; 3];
    let mut overall = f32::MAX;
    let mut winding = 0;
    for piece in coloured {
        let squared = piece.segment.distance_squared(x, y);
        overall = overall.min(squared);
        for (channel, closest) in nearest.iter_mut().enumerate() {
            if piece.channels.holds(channel) {
                *closest = closest.min(squared);
            }
        }
        winding += piece.segment.winding(x, y);
    }
    let sign = if winding != 0 { -1.0 } else { 1.0 };
    (
        [
            nearest[0].sqrt() * sign,
            nearest[1].sqrt() * sign,
            nearest[2].sqrt() * sign,
        ],
        overall.sqrt() * sign,
    )
}
