use morf_layout::Layout;
use morf_region::{Operation, Shape};
use morf_scene::{Color, Element, NodeHandle, Scene};

use crate::{commands::*, effects::*, sdf::*};

/// What a field hands down to every layer that does not speak for itself.
///
/// The container owns the compound: one blend, one fill, one position along the
/// morph. A layer opts out by naming its own.
#[derive(Clone, Copy)]
pub(crate) struct FieldDefaults {
    pub(crate) blend: f32,
    pub(crate) color: Color,
    pub(crate) morph: f32,
    /// The tint inherited from an ancestor.
    ///
    /// The field's own fill has always had this composited in; its layers did
    /// not, so tinting a subtree changed a field that fell back to the field
    /// colour and left alone every layer that named its own — half a field
    /// taking the tint and half of it ignoring it.
    pub(crate) overlay: Color,
}

/// Reads everything beneath a field that has a shape, in composition order.
///
/// This is what makes fields the foundation rather than a separate kind of
/// drawing: an ordinary `Rect` under an `Sdf` becomes a rounded-box layer, and
/// the walk descends through the positioners, so a `Row` of rects laid out by
/// the normal layout engine arrives here as a row of fields to fuse. Anything
/// without a field of its own — text, images, a mouse area — is left alone and
/// paints over the composition as usual.
///
/// A nested `Sdf` is not descended into. It is its own composition with its own
/// fill, and folding its layers into the parent would silently discard that.
pub(crate) fn field_layers(
    scene: &Scene,
    layout: &Layout,
    node: NodeHandle,
    defaults: FieldDefaults,
    layers: &mut Vec<SdfLayer>,
) -> Result<(), RenderError> {
    for &child in scene.children(node)? {
        if !scene.bool_value(child, "visible")? {
            continue;
        }
        match scene.element(child)? {
            Element::SdfShape => {
                if let Some(layer) = shape_layer(scene, layout, child, defaults)? {
                    layers.push(layer);
                }
            }
            Element::Rect | Element::ClipRect => {
                if let Some(layer) = rect_layer(scene, layout, child, defaults)? {
                    layers.push(layer);
                }
                // A rect may still position children, and those children are
                // part of the same composition.
                field_layers(scene, layout, child, defaults, layers)?;
            }
            Element::Sdf => {}
            _ => field_layers(scene, layout, child, defaults, layers)?,
        }
    }
    Ok(())
}

/// Whether a node's own paint is absorbed by an enclosing field.
///
/// A rect that became a layer must not also be drawn as a rect, or the
/// composition is painted twice — once fused and once with every seam back.
pub(crate) fn absorbed_by_field(element: Element) -> bool {
    matches!(
        element,
        Element::Rect | Element::ClipRect | Element::SdfShape
    )
}

/// Reads one `SdfShape` into a layer.
fn shape_layer(
    scene: &Scene,
    layout: &Layout,
    node: NodeHandle,
    defaults: FieldDefaults,
) -> Result<Option<SdfLayer>, RenderError> {
    let Some(bounds) = layout.geometry(node) else {
        return Ok(None);
    };
    // A named letter decides the family: it is one particular outline, not a
    // shape with parameters, so there is nothing for `shape` to say about it.
    let glyph = scene.string_value(node, "glyph")?.chars().next();
    let glyph_morph_to = scene.string_value(node, "glyph_morph_to")?.chars().next();
    let name = scene.string_value(node, "shape")?;
    let named = Shape::parse(name)
        .ok_or_else(|| RenderError::Scene(format!("unknown SdfShape shape `{name}`")))?;
    // Naming a letter is enough to mean the layer *is* that letter, unless the
    // shape was named too — which is how a shape morphs into a letter rather
    // than out of one.
    let shape = if glyph.is_some() && name == "circle" {
        Shape::Polygon
    } else {
        named
    };
    let target = scene.string_value(node, "morph_to")?;
    let morph_to = if target.is_empty() {
        shape
    } else {
        Shape::parse(target)
            .ok_or_else(|| RenderError::Scene(format!("unknown SdfShape shape `{target}`")))?
    };
    let operation = scene.string_value(node, "operation")?;
    let operation = Operation::parse(operation)
        .ok_or_else(|| RenderError::Scene(format!("unknown SdfShape operation `{operation}`")))?;
    Ok(Some(SdfLayer {
        glyph,
        glyph_morph_to,
        // Only a letter has a face. Asking for the string when there is no
        // glyph would allocate on every plain shape in every field, every
        // frame, to describe something that is never read.
        font_family: match glyph {
            Some(_) => Some(scene.string_value(node, "font_family")?.into()),
            None => None,
        },
        font_family_morph_to: match glyph {
            Some(_) => match scene.string_value(node, "font_family_morph_to")? {
                "" => None,
                named => Some(named.into()),
            },
            None => None,
        },
        bounds,
        color: layer_color(scene, node, defaults)?,
        shape,
        morph_to,
        morph: {
            let own = scene.number(node, "morph_progress")?;
            if own < 0.0 {
                defaults.morph
            } else {
                own as f32
            }
        }
        .clamp(0.0, 1.0),
        operation,
        blend: layer_blend(scene, node, defaults.blend)?,
        rotation: scene.number(node, "rotation")? as f32,
        radii: rect_radii(scene, node)?.map(|radius| radius as f32),
        points: scene.number(node, "points")?.clamp(3.0, 64.0) as f32,
        inner_radius: scene.number(node, "inner_radius")?.clamp(0.01, 1.0) as f32,
        thickness: scene.number(node, "thickness")?.max(0.0) as f32,
        angle: scene.number(node, "angle")?.clamp(0.0, 360.0) as f32,
    }))
}

/// Reads an ordinary rect into a rounded-box layer.
///
/// A field box carries one corner radius where a rect carries four, so the
/// largest of them is used: a rect that is round on one corner reads as round
/// rather than square once it is part of a fused surface.
fn rect_layer(
    scene: &Scene,
    layout: &Layout,
    node: NodeHandle,
    defaults: FieldDefaults,
) -> Result<Option<SdfLayer>, RenderError> {
    let Some(bounds) = layout.geometry(node) else {
        return Ok(None);
    };
    if bounds.width <= 0.0 || bounds.height <= 0.0 {
        return Ok(None);
    }
    let blend = layer_blend(scene, node, defaults.blend)?;
    Ok(Some(SdfLayer {
        glyph: None,
        glyph_morph_to: None,
        font_family: None,
        font_family_morph_to: None,
        bounds,
        // A rect brings its own colour into the composition, so a fused row of
        // differently coloured rects keeps every one of them and blends across
        // the seams rather than flattening to a single fill.
        color: scene
            .color_value(node, "color")
            .map_or(defaults.color, |color| {
                apply_overlay(color, defaults.overlay)
            }),
        shape: Shape::Box,
        morph_to: Shape::Box,
        morph: 0.0,
        // A rect joins what is already there, smoothly when the field asks for
        // it. Nothing about the rect had to be written differently to take
        // part; it is the container that decides.
        operation: if blend > 0.0 {
            Operation::SmoothUnion
        } else {
            Operation::Union
        },
        blend,
        rotation: scene.number(node, "rotation")? as f32,
        // All four, so a rect rounded on one edge keeps that shape once it is
        // part of a fused surface instead of collapsing to a single radius.
        radii: rect_radii(scene, node)?.map(|radius| radius as f32),
        points: 5.0,
        inner_radius: 0.5,
        thickness: 0.0,
        angle: 90.0,
    }))
}

/// A layer's own fill, falling back to the field's when it names none.
///
/// Transparent is the sentinel rather than a missing property, so a layer that
/// wants the field's colour simply says nothing.
fn layer_color(
    scene: &Scene,
    node: NodeHandle,
    defaults: FieldDefaults,
) -> Result<Color, RenderError> {
    let own = scene.color_value(node, "fill_color")?;
    // The default already carries the overlay; a layer's own colour has to have
    // it applied here, so both answers are tinted the same way.
    Ok(if own.alpha > 0.0 {
        apply_overlay(own, defaults.overlay)
    } else {
        defaults.color
    })
}

/// A layer's own seam radius, falling back to the field's.
fn layer_blend(scene: &Scene, node: NodeHandle, default_blend: f32) -> Result<f32, RenderError> {
    let own = scene.number(node, "blend").unwrap_or(0.0).max(0.0) as f32;
    Ok(if own > 0.0 { own } else { default_blend })
}
