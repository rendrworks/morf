//! Where a child sits across the axis its positioner packs along.

use morf_scene::Element;

use crate::geometry::Geometry;
use crate::helpers::LayoutError;

/// Places a child across the axis its parent packs along.
pub(crate) fn align_across(
    parent_element: Element,
    alignment: &str,
    parent_geometry: Geometry,
    geometry: &mut Geometry,
) -> Result<(), LayoutError> {
    let horizontal = parent_element == Element::Row;
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
