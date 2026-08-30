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
fn field_layers(
    scene: &Scene,
    layout: &Layout,
    node: NodeHandle,
    default_blend: f32,
    default_color: Color,
    layers: &mut Vec<SdfLayer>,
) -> Result<(), RenderError> {
    for child in scene.children(node)? {
        if !scene.bool_value(child, "visible")? {
            continue;
        }
        match scene.element(child)? {
            Element::SdfShape => {
                if let Some(layer) =
                    shape_layer(scene, layout, child, default_blend, default_color)?
                {
                    layers.push(layer);
                }
            }
            Element::Rect | Element::ClipRect => {
                if let Some(layer) = rect_layer(scene, layout, child, default_blend, default_color)?
                {
                    layers.push(layer);
                }
                // A rect may still position children, and those children are
                // part of the same composition.
                field_layers(scene, layout, child, default_blend, default_color, layers)?;
            }
            Element::Sdf => {}
            _ => field_layers(scene, layout, child, default_blend, default_color, layers)?,
        }
    }
    Ok(())
}

/// Whether a node's own paint is absorbed by an enclosing field.
///
/// A rect that became a layer must not also be drawn as a rect, or the
/// composition is painted twice — once fused and once with every seam back.
fn absorbed_by_field(element: Element) -> bool {
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
    default_blend: f32,
    default_color: Color,
) -> Result<Option<SdfLayer>, RenderError> {
    let Some(bounds) = layout.geometry(node) else {
        return Ok(None);
    };
    let name = scene.string_value(node, "shape")?;
    let shape = SdfShapeKind::parse(name)
        .ok_or_else(|| RenderError::Scene(format!("unknown SdfShape shape `{name}`")))?;
    let target = scene.string_value(node, "morph_to")?;
    let morph_to = if target.is_empty() {
        shape
    } else {
        SdfShapeKind::parse(target)
            .ok_or_else(|| RenderError::Scene(format!("unknown SdfShape shape `{target}`")))?
    };
    let operation = scene.string_value(node, "operation")?;
    let operation = SdfOperation::parse(operation)
        .ok_or_else(|| RenderError::Scene(format!("unknown SdfShape operation `{operation}`")))?;
    Ok(Some(SdfLayer {
        bounds,
        color: layer_color(scene, node, default_color)?,
        shape,
        morph_to,
        morph: scene.number(node, "morph_progress")?.clamp(0.0, 1.0) as f32,
        operation,
        blend: layer_blend(scene, node, default_blend)?,
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
    default_blend: f32,
    default_color: Color,
) -> Result<Option<SdfLayer>, RenderError> {
    let Some(bounds) = layout.geometry(node) else {
        return Ok(None);
    };
    if bounds.width <= 0.0 || bounds.height <= 0.0 {
        return Ok(None);
    }
    let blend = layer_blend(scene, node, default_blend)?;
    Ok(Some(SdfLayer {
        bounds,
        // A rect brings its own colour into the composition, so a fused row of
        // differently coloured rects keeps every one of them and blends across
        // the seams rather than flattening to a single fill.
        color: scene.color_value(node, "color").unwrap_or(default_color),
        shape: SdfShapeKind::Box,
        morph_to: SdfShapeKind::Box,
        morph: 0.0,
        // A rect joins what is already there, smoothly when the field asks for
        // it. Nothing about the rect had to be written differently to take
        // part; it is the container that decides.
        operation: if blend > 0.0 {
            SdfOperation::SmoothUnion
        } else {
            SdfOperation::Union
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
    default_color: Color,
) -> Result<Color, RenderError> {
    let own = scene.color_value(node, "fill_color")?;
    Ok(if own.alpha > 0.0 { own } else { default_color })
}

/// A layer's own seam radius, falling back to the field's.
fn layer_blend(scene: &Scene, node: NodeHandle, default_blend: f32) -> Result<f32, RenderError> {
    let own = scene.number(node, "blend").unwrap_or(0.0).max(0.0) as f32;
    Ok(if own > 0.0 { own } else { default_blend })
}
