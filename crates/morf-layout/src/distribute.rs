//! How a layout container shares its space.
//!
//! Split from `layout` at the line gate. Two questions a `RowLayout` or
//! `ColumnLayout` answers for each child that a plain positioner does not:
//! how much of the leftover space it gets, and where it sits across the
//! packed axis.

use std::collections::{BTreeMap, HashMap};

use morf_scene::{Element, FastMap, NodeHandle, Scene, Value};

use crate::geometry::{Geometry, Size};
use crate::helpers::{LayoutError, attached_layout, flag, layout_string, layout_weight};

/// How much each child of a layout grows (or, negative, shrinks) along the
/// packed axis.
///
/// Leftover space goes to the fillers in proportion to their
/// `layout.stretch` (one by default); a shortfall comes out of every child
/// in proportion to `layout.shrink` times its size, down to its minimum,
/// which is what flexbox does and what a bar with a long title in it
/// needs. Plain positioners get an empty map.
pub(crate) fn distribute(
    requested: &FastMap<NodeHandle, Size>,
    scene: &Scene,
    parent_element: Element,
    children: &[NodeHandle],
    spacing: f64,
    parent_geometry: Geometry,
) -> Result<HashMap<NodeHandle, f64>, LayoutError> {
    let mut growth = HashMap::new();
    if matches!(parent_element, Element::RowLayout | Element::ColumnLayout) {
        let horizontal = parent_element == Element::RowLayout;
        let mut occupied = spacing * children.len().saturating_sub(1) as f64;
        // Leftover space goes to the fillers in proportion to their
        // `layout.stretch` (one by default); a shortfall comes out of
        // every child in proportion to `layout.shrink` times its size,
        // down to its minimum, which is what flexbox does and what a
        // bar with a long title in it needs.
        let mut fillers = Vec::new();
        let mut shrinkers = Vec::new();
        for &child in children {
            let size = requested[&child];
            let extent = if horizontal { size.width } else { size.height };
            occupied += extent;
            let attached = attached_layout(scene.current(child, "layout")?)?;
            if flag(
                attached,
                if horizontal {
                    "fill_width"
                } else {
                    "fill_height"
                },
            ) {
                fillers.push((child, layout_weight(attached, "stretch", 1.0)));
            }
            let shrink = layout_weight(attached, "shrink", 1.0);
            let minimum = layout_weight(
                attached,
                if horizontal {
                    "minimum_width"
                } else {
                    "minimum_height"
                },
                0.0,
            );
            shrinkers.push((child, shrink * extent, (extent - minimum).max(0.0)));
        }
        let available = if horizontal {
            parent_geometry.width
        } else {
            parent_geometry.height
        };
        if available >= occupied {
            let weights = fillers.iter().map(|(_, weight)| weight).sum::<f64>();
            for (child, weight) in fillers {
                let share = if weights > 0.0 {
                    (available - occupied) * weight / weights
                } else {
                    0.0
                };
                growth.insert(child, share);
            }
        } else {
            let deficit = occupied - available;
            let weights = shrinkers.iter().map(|(_, weight, _)| weight).sum::<f64>();
            for (child, weight, room) in shrinkers {
                if weights > 0.0 {
                    growth.insert(child, -(deficit * weight / weights).min(room));
                }
            }
        }
    }
    Ok(growth)
}

/// Places a child across the axis its parent packs along.
pub(crate) fn align_across(
    parent_element: Element,
    alignment: &str,
    attached: &BTreeMap<String, Value>,
    parent_geometry: Geometry,
    geometry: &mut Geometry,
) -> Result<(), LayoutError> {
    let alignment = layout_string(attached, "alignment").unwrap_or(alignment);
    let horizontal = matches!(parent_element, Element::Row | Element::RowLayout);
    let extent = if horizontal {
        parent_geometry.height
    } else {
        parent_geometry.width
    };
    let own = if horizontal {
        geometry.height
    } else {
        geometry.width
    };
    let offset = match alignment {
        "center" => Some((extent - own) / 2.0),
        "end" => Some(extent - own),
        "stretch" => {
            if horizontal {
                geometry.height = extent;
            } else {
                geometry.width = extent;
            }
            Some(0.0)
        }
        "start" => None,
        other => {
            return Err(LayoutError::Scene(format!(
                "unknown alignment `{other}`: use start, center, end or stretch"
            )));
        }
    };
    if let Some(offset) = offset {
        if horizontal {
            geometry.y = offset;
        } else {
            geometry.x = offset;
        }
    }
    Ok(())
}
