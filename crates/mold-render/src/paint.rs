#[derive(Clone, Copy)]
struct PaintContext {
    opacity: f64,
    transform: Transform2D,
    clip: Option<Geometry>,
    overlay: Color,
    layer: Option<usize>,
    /// Whether an enclosing field has already taken this node's shape.
    in_field: bool,
}

fn append_node(
    scene: &Scene,
    layout: &Layout,
    node: NodeHandle,
    inherited: PaintContext,
    list: &mut DrawList,
) -> Result<(), RenderError> {
    if !scene.bool_value(node, "visible")? {
        return Ok(());
    }
    let node_opacity = scene.number(node, "opacity")?.clamp(0.0, 1.0);
    let rotation = scene.number(node, "rotation")?;
    let layer_config = layer_config(scene, node)?;
    let element = scene.element(node)?;
    let rect_blur = if matches!(element, Element::Rect | Element::ClipRect) {
        scene.number(node, "blur")?.max(0.0)
    } else {
        0.0
    };
    let layer_blur = layer_config.blur.max(rect_blur);
    let rounded_clip = scene.bool_value(node, "clip")?
        && matches!(element, Element::Rect | Element::ClipRect)
        && rect_radii(scene, node)?.iter().any(|radius| *radius > 0.0);
    let creates_layer = layer_config.enabled
        || node_opacity < 1.0
        || rotation != 0.0
        || rounded_clip
        || layer_blur > 0.0
        || layer_config.shadow_color.alpha > 0.0;
    let layer = creates_layer.then(|| {
        let index = list.layers.len();
        list.layers.push(Layer {
            node,
            commands: list.commands.len()..list.commands.len(),
            parent: inherited.layer,
            opacity: node_opacity as f32,
            blur: layer_blur as f32,
            shadow_color: layer_config.shadow_color,
            shadow_blur: layer_config.shadow_blur as f32,
            shadow_offset: [
                layer_config.shadow_offset_x as f32,
                layer_config.shadow_offset_y as f32,
            ],
            mask: None,
            bounds: Geometry::default(),
        });
        index
    });
    let opacity = if creates_layer {
        inherited.opacity
    } else {
        inherited.opacity * node_opacity
    };
    let color_overlay =
        compose_overlay(inherited.overlay, scene.color_value(node, "color_overlay")?);
    let Some(bounds) = layout.geometry(node) else {
        return Ok(());
    };
    let transform = inherited.transform.then(
        node_transform(scene, node, bounds)
            .map_err(|error| RenderError::Scene(error.to_string()))?,
    );
    if let Some(layer) = layer
        && rounded_clip
    {
        list.layers[layer].mask = Some(LayerMask {
            bounds,
            transform,
            radii: rect_radii(scene, node)?,
        });
    }
    let clip = if scene.bool_value(node, "clip")? {
        let bounds = transform.bounds(bounds);
        Some(
            inherited
                .clip
                .map_or(bounds, |inherited| intersect_geometry(inherited, bounds)),
        )
    } else {
        inherited.clip
    };
    // A shape an enclosing field composed is drawn by that field, not again on
    // its own. Everything else — text, images, anything without a field —
    // paints normally over the composition.
    let painted = !(inherited.in_field && absorbed_by_field(element));
    match element {
        Element::Rect | Element::ClipRect if painted => list.commands.push(DrawCommand::Quad {
            node,
            bounds,
            transform,
            clip,
            color: with_opacity(scene.color_value(node, "color")?, opacity),
            color_overlay,
            gradient: scene_gradient(scene, node, opacity)?,
            radii: rect_radii(scene, node)?,
            border_width: if element == Element::ClipRect {
                0.0
            } else {
                scene.number(node, "border_width")?
            },
            antialiasing: element != Element::ClipRect || scene.bool_value(node, "antialiasing")?,
            border_pixel_aligned: element == Element::ClipRect
                && scene.bool_value(node, "border_pixel_aligned")?,
            border_color: with_opacity(scene.color_value(node, "border_color")?, opacity),
            blur: if layer_blur > 0.0 { 0.0 } else { rect_blur },
            shadow_color: with_opacity(scene.color_value(node, "shadow_color")?, opacity),
            shadow_blur: scene.number(node, "shadow_blur")?.max(0.0),
            shadow_spread: scene.number(node, "shadow_spread")?,
            shadow_offset_x: scene.number(node, "shadow_offset_x")?,
            shadow_offset_y: scene.number(node, "shadow_offset_y")?,
            shadow_inner: scene.bool_value(node, "shadow_inner")?,
        }),
        Element::Text => list.commands.push(DrawCommand::Text {
            node,
            bounds,
            transform,
            clip,
            text: scene.string_value(node, "text")?.to_owned(),
            family: scene.string_value(node, "font_family")?.to_owned(),
            font_source: scene.string_value(node, "font_source")?.to_owned(),
            size: scene.number(node, "font_size")?,
            font_weight: scene.number(node, "font_weight")?,
            color: with_opacity(scene.color_value(node, "color")?, opacity),
            color_overlay,
            wrap: scene.bool_value(node, "wrap")?,
            elide: render_text_elide(scene.string_value(node, "elide")?)?,
            horizontal_alignment: render_text_alignment(
                scene.string_value(node, "horizontal_alignment")?,
            )?,
            vertical_alignment: vertical_alignment(
                scene.string_value(node, "vertical_alignment")?,
            )?,
        }),
        Element::Image => list.commands.push(DrawCommand::Texture {
            node,
            bounds,
            transform,
            clip,
            source: scene.string_value(node, "source")?.to_owned(),
            icon_theme: None,
            opacity: opacity as f32,
            color_overlay,
            fill_mode: image_fill_mode(scene.string_value(node, "fill_mode")?)?,
            distance_field: scene.bool_value(node, "distance_field")?,
            distance_field_spread: scene.number(node, "distance_field_spread")?.max(0.5) as f32,
            distance_field_style: distance_field_style(scene, node, opacity)?,
        }),
        Element::Icon => list.commands.push(DrawCommand::Texture {
            node,
            bounds,
            transform,
            clip,
            source: scene.string_value(node, "name")?.to_owned(),
            icon_theme: Some(scene.string_value(node, "theme")?.to_owned()),
            opacity: opacity as f32,
            color_overlay,
            fill_mode: image_fill_mode(scene.string_value(node, "fill_mode")?)?,
            distance_field: scene.bool_value(node, "distance_field")?,
            distance_field_spread: scene.number(node, "distance_field_spread")?.max(0.5) as f32,
            distance_field_style: distance_field_style(scene, node, opacity)?,
        }),
        Element::Shape => list.commands.push(DrawCommand::Path {
            node,
            bounds,
            transform,
            clip,
            path: scene.string_value(node, "path")?.to_owned(),
            fill_color: apply_overlay(
                with_opacity(scene.color_value(node, "fill_color")?, opacity),
                color_overlay,
            ),
            stroke_color: apply_overlay(
                with_opacity(scene.color_value(node, "stroke_color")?, opacity),
                color_overlay,
            ),
            stroke_width: scene.number(node, "stroke_width")?.max(0.0),
            even_odd: scene.string_value(node, "fill_rule")? == "even_odd",
        }),
        Element::Sdf if painted => {
            let mut layers = Vec::new();
            let blend = scene.number(node, "blend")?.max(0.0) as f32;
            let fill = apply_overlay(
                with_opacity(scene.color_value(node, "fill_color")?, opacity),
                color_overlay,
            );
            field_layers(scene, layout, node, blend, fill, &mut layers)?;
            // A composition with nothing in it has no zero crossing and would
            // paint the whole rectangle, so it draws nothing at all.
            if !layers.is_empty() {
                list.commands.push(DrawCommand::Field {
                    node,
                    bounds,
                    transform,
                    clip,
                    fill_color: apply_overlay(
                        with_opacity(scene.color_value(node, "fill_color")?, opacity),
                        color_overlay,
                    ),
                    stroke_color: apply_overlay(
                        with_opacity(scene.color_value(node, "stroke_color")?, opacity),
                        color_overlay,
                    ),
                    stroke_width: scene.number(node, "stroke_width")?.max(0.0),
                    softness: scene.number(node, "softness")?.max(0.0),
                    layers,
                });
            }
        }
        Element::Rect
        | Element::ClipRect
        | Element::Sdf
        | Element::Item
        | Element::Inset
        | Element::SdfShape
        | Element::MouseArea
        | Element::Row
        | Element::Column
        | Element::Grid
        | Element::RowLayout
        | Element::ColumnLayout
        | Element::GridLayout
        | Element::Flickable
        | Element::Loader
        | Element::Timer => {}
    }
    let content_layer = if element == Element::ClipRect
        && scene.number(node, "border_width")? > 0.0
        && !scene.bool_value(node, "content_under_border")?
    {
        let border = scene.number(node, "border_width")?.max(0.0);
        let inner = Geometry {
            x: bounds.x + border,
            y: bounds.y + border,
            width: (bounds.width - border * 2.0).max(0.0),
            height: (bounds.height - border * 2.0).max(0.0),
        };
        let index = list.layers.len();
        list.layers.push(Layer {
            node,
            commands: list.commands.len()..list.commands.len(),
            parent: layer.or(inherited.layer),
            opacity: 1.0,
            blur: 0.0,
            shadow_color: Color::rgba8(0, 0, 0, 0),
            shadow_blur: 0.0,
            shadow_offset: [0.0, 0.0],
            mask: Some(LayerMask {
                bounds: inner,
                transform,
                radii: rect_radii(scene, node)?.map(|radius| (radius - border).max(0.0)),
            }),
            bounds: inner,
        });
        Some((index, inner))
    } else {
        None
    };
    for child in scene.children(node)? {
        let child_clip = content_layer.map_or(clip, |(_, inner)| {
            let inner = transform.bounds(inner);
            Some(clip.map_or(inner, |clip| intersect_geometry(clip, inner)))
        });
        append_node(
            scene,
            layout,
            child,
            PaintContext {
                opacity,
                transform,
                clip: child_clip,
                overlay: color_overlay,
                layer: content_layer
                    .map(|(layer, _)| layer)
                    .or(layer)
                    .or(inherited.layer),
                // A field claims every shape beneath it, however deeply the
                // positioners nest, which is what lets an ordinary laid-out
                // row of rects arrive as one fused surface.
                in_field: inherited.in_field || element == Element::Sdf,
            },
            list,
        )?;
    }
    if let Some((content_layer, inner)) = content_layer {
        list.layers[content_layer].commands.end = list.commands.len();
        list.layers[content_layer].bounds = inner;
    }
    if element == Element::ClipRect && scene.number(node, "border_width")? > 0.0 {
        list.commands.push(DrawCommand::Quad {
            node,
            bounds,
            transform,
            clip,
            color: Color::rgba8(0, 0, 0, 0),
            color_overlay: Color::rgba8(0, 0, 0, 0),
            gradient: Gradient::None,
            radii: rect_radii(scene, node)?,
            border_width: scene.number(node, "border_width")?,
            antialiasing: scene.bool_value(node, "antialiasing")?,
            border_pixel_aligned: scene.bool_value(node, "border_pixel_aligned")?,
            border_color: apply_overlay(
                with_opacity(scene.color_value(node, "border_color")?, opacity),
                color_overlay,
            ),
            blur: 0.0,
            shadow_color: Color::rgba8(0, 0, 0, 0),
            shadow_blur: 0.0,
            shadow_spread: 0.0,
            shadow_offset_x: 0.0,
            shadow_offset_y: 0.0,
            shadow_inner: false,
        });
    }
    if let Some(layer) = layer {
        let start = list.layers[layer].commands.start;
        let end = list.commands.len();
        list.layers[layer].commands.end = end;
        let bounds =
            command_union(&list.commands[start..end]).unwrap_or_else(|| transform.bounds(bounds));
        let blurred = expand_geometry(bounds, layer_blur * 2.0);
        list.layers[layer].bounds = if layer_config.shadow_color.alpha > 0.0 {
            union_geometry(
                blurred,
                offset_geometry(
                    expand_geometry(bounds, layer_config.shadow_blur * 2.0),
                    layer_config.shadow_offset_x,
                    layer_config.shadow_offset_y,
                ),
            )
        } else {
            blurred
        };
    }
    Ok(())
}

/// Reads the animatable edge shaping applied to a cached distance field.
///
/// The outline colour carries node opacity like every other painted colour, so
/// fading an icon out takes its outline with it.
fn distance_field_style(
    scene: &Scene,
    node: NodeHandle,
    opacity: f64,
) -> Result<DistanceFieldStyle, RenderError> {
    Ok(DistanceFieldStyle {
        weight: scene.number(node, "distance_field_weight")?.clamp(0.0, 1.0) as f32,
        softness: scene.number(node, "distance_field_softness")?.max(0.0) as f32,
        outline_width: scene.number(node, "distance_field_outline_width")?.max(0.0) as f32,
        outline_color: with_opacity(
            scene.color_value(node, "distance_field_outline_color")?,
            opacity,
        ),
    })
}
