use morf_layout::{Geometry, Layout, Transform2D, node_transform};
use morf_scene::{Color, Element, NodeHandle, Scene};

use crate::{commands::*, effects::*, paint_fields::*, sdf::*};

#[derive(Clone, Copy)]
pub(crate) struct PaintContext {
    pub(crate) transform: Transform2D,
    pub(crate) clip: Option<Geometry>,
    pub(crate) overlay: Color,
    pub(crate) layer: Option<usize>,
    /// Whether an enclosing field has already taken this node's shape.
    pub(crate) in_field: bool,
}

pub(crate) fn append_node(
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
    // Both are asked for more than once further down, and each `rect_radii` is
    // five property reads of its own.
    let clips = scene.bool_value(node, "clip")?;
    let radii = if matches!(element, Element::Rect | Element::ClipRect) {
        rect_radii(scene, node)?
    } else {
        [0.0; 4]
    };
    let rounded_clip = clips
        && matches!(element, Element::Rect | Element::ClipRect)
        && radii.iter().any(|radius| *radius > 0.0);
    // An effect shader composites the subtree rather than colouring one node,
    // so it is taken off the node here and given to the layer below.
    let effect = shader_binding(scene, node)?.filter(|shader| shader.samples_behind);
    // A shape an enclosing field composes is not drawn as a node at all — its
    // rotation is one of the numbers the field is given, and the field turns
    // the sample point by it. Making a layer to rotate it into would be an
    // offscreen target for a subtree that paints nothing.
    //
    // It was also wrong, not merely wasteful. The layer came out empty, an
    // empty layer claims the index of the command that would have followed it,
    // and the frame loop then stepped over that command: a rotated shape inside
    // a field silently ate whatever was drawn next.
    let absorbed = inherited.in_field && absorbed_by_field(element);
    let creates_layer = layer_config.enabled
        || node_opacity < 1.0
        || (rotation != 0.0 && !absorbed)
        || rounded_clip
        || layer_blur > 0.0
        || layer_config.shadow_color.alpha > 0.0
        // An effect shader has nothing to read until its subtree has been
        // rendered somewhere, so a node carrying one becomes a layer whether or
        // not anything else would have made it into one.
        || effect.is_some();
    let layer = creates_layer.then(|| {
        let index = list.layers.len();
        list.layers.push(Layer {
            node,
            shader: effect.clone(),
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
            radii,
        });
    }
    let clip = if clips {
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
    let painted = !absorbed;
    // A shadow is five numbers and a flag that only matter once the colour is
    // visible, and a rect with no shadow is the overwhelming majority. Asking
    // for the colour first turns six property reads into one for all of them.
    let shadow = if painted && matches!(element, Element::Rect | Element::ClipRect) {
        rect_shadow(scene, node)?
    } else {
        RectShadow::none()
    };
    match element {
        Element::Rect | Element::ClipRect if painted => list.commands.push(DrawCommand::Quad {
            node,
            bounds,
            transform,
            clip,
            color: scene.color_value(node, "color")?,
            color_overlay,
            gradient: scene_gradient(scene, node)?,
            radii,
            border_width: if element == Element::ClipRect {
                0.0
            } else {
                scene.number(node, "border_width")?
            },
            antialiasing: element != Element::ClipRect || scene.bool_value(node, "antialiasing")?,
            border_pixel_aligned: element == Element::ClipRect
                && scene.bool_value(node, "border_pixel_aligned")?,
            border_color: scene.color_value(node, "border_color")?,
            blur: if layer_blur > 0.0 { 0.0 } else { rect_blur },
            shadow_color: shadow.color,
            shadow_blur: shadow.blur,
            shadow_spread: shadow.spread,
            shadow_offset_x: shadow.offset_x,
            shadow_offset_y: shadow.offset_y,
            shadow_inner: shadow.inner,
            // A rectangle wears a shader the same way a field does, because it
            // *is* a field of one layer. An effect shader belongs to the layer
            // that composites the node, not to its own fill.
            shader: shader_binding(scene, node)?.filter(|shader| !shader.samples_behind),
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
            color: scene.color_value(node, "color")?,
            color_overlay,
            wrap: scene.bool_value(node, "wrap")?,
            max_lines: scene.number(node, "max_lines")?.max(0.0) as usize,
            elide: render_text_elide(scene.string_value(node, "elide")?)?,
            horizontal_alignment: render_text_alignment(
                scene.string_value(node, "horizontal_alignment")?,
            )?,
            vertical_alignment: vertical_alignment(
                scene.string_value(node, "vertical_alignment")?,
            )?,
            field_style: text_field_style(scene, node)?,
            morph_to: scene.string_value(node, "morph_to")?.to_owned(),
            morph_progress: scene.number(node, "morph_progress")?.clamp(0.0, 1.0) as f32,
        }),
        Element::Image => list.commands.push(DrawCommand::Texture {
            node,
            bounds,
            transform,
            clip,
            source: scene.string_value(node, "source")?.to_owned(),
            icon_theme: None,
            color_overlay,
            fill_mode: image_fill_mode(scene.string_value(node, "fill_mode")?)?,
            distance_field: scene.bool_value(node, "distance_field")?,
            distance_field_spread: scene.number(node, "distance_field_spread")?.max(0.5) as f32,
            distance_field_style: text_field_style(scene, node)?,
        }),
        Element::Icon => list.commands.push(DrawCommand::Texture {
            node,
            bounds,
            transform,
            clip,
            source: scene.string_value(node, "name")?.to_owned(),
            icon_theme: Some(scene.string_value(node, "theme")?.to_owned()),
            color_overlay,
            fill_mode: image_fill_mode(scene.string_value(node, "fill_mode")?)?,
            distance_field: scene.bool_value(node, "distance_field")?,
            distance_field_spread: scene.number(node, "distance_field_spread")?.max(0.5) as f32,
            distance_field_style: text_field_style(scene, node)?,
        }),
        Element::Sdf if painted => {
            let mut layers = Vec::new();
            let defaults = FieldDefaults {
                blend: scene.number(node, "blend")?.max(0.0) as f32,
                color: apply_overlay(scene.color_value(node, "fill_color")?, color_overlay),
                morph: scene.number(node, "morph_progress")?.clamp(0.0, 1.0) as f32,
                overlay: color_overlay,
            };
            field_layers(scene, layout, node, defaults, &mut layers)?;
            // A composition with nothing in it has no zero crossing and would
            // paint the whole rectangle, so it draws nothing at all.
            if !layers.is_empty() {
                list.commands.push(DrawCommand::Field {
                    node,
                    bounds,
                    transform,
                    clip,
                    fill_color: apply_overlay(
                        scene.color_value(node, "fill_color")?,
                        color_overlay,
                    ),
                    stroke_color: apply_overlay(
                        scene.color_value(node, "stroke_color")?,
                        color_overlay,
                    ),
                    stroke_width: scene.number(node, "stroke_width")?.max(0.0),
                    stroke_alignment: stroke_alignment(
                        scene.string_value(node, "stroke_alignment")?,
                    )?,
                    softness: scene.number(node, "softness")?.max(0.0),
                    gradient: scene_gradient(scene, node)?,
                    color_overlay,
                    shadow_color: scene.color_value(node, "shadow_color")?,
                    shadow_blur: scene.number(node, "shadow_blur")?.max(0.0),
                    shadow_spread: scene.number(node, "shadow_spread")?,
                    shadow_offset_x: scene.number(node, "shadow_offset_x")?,
                    shadow_offset_y: scene.number(node, "shadow_offset_y")?,
                    shadow_inner: scene.bool_value(node, "shadow_inner")?,
                    // An effect shader belongs to the layer that composites
                    // this node, not to the node's own fill: leaving it here
                    // too would have the field pass look for a program that
                    // was registered against the composite pass.
                    shader: shader_binding(scene, node)?.filter(|shader| !shader.samples_behind),
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
        | Element::Timer
        | Element::Flex => {}
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
            shader: None,
            mask: Some(LayerMask {
                bounds: inner,
                transform,
                radii: radii.map(|radius| (radius - border).max(0.0)),
            }),
            bounds: inner,
        });
        Some((index, inner))
    } else {
        None
    };
    for &child in scene.paint_order(node)?.iter() {
        let child_clip = content_layer.map_or(clip, |(_, inner)| {
            let inner = transform.bounds(inner);
            Some(clip.map_or(inner, |clip| intersect_geometry(clip, inner)))
        });
        append_node(
            scene,
            layout,
            child,
            PaintContext {
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
            radii,
            border_width: scene.number(node, "border_width")?,
            antialiasing: scene.bool_value(node, "antialiasing")?,
            border_pixel_aligned: scene.bool_value(node, "border_pixel_aligned")?,
            border_color: apply_overlay(scene.color_value(node, "border_color")?, color_overlay),
            blur: 0.0,
            shadow_color: Color::rgba8(0, 0, 0, 0),
            shadow_blur: 0.0,
            shadow_spread: 0.0,
            shadow_offset_x: 0.0,
            shadow_offset_y: 0.0,
            shadow_inner: false,
            // The border a ClipRect overlays is not the node's own fill, so it
            // carries no shader: the shader belongs to what is inside.
            shader: None,
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

/// A rect's drop shadow, read only as far as it is visible.
/// How a text node wants its glyph fields thresholded.
///
/// Thickness is in logical pixels of edge movement, which is what a
/// configuration can reason about: asking for half a pixel more weight means
/// the same thing at every size, where a shift in field units would not.
fn text_field_style(scene: &Scene, node: NodeHandle) -> Result<DistanceFieldStyle, RenderError> {
    Ok(DistanceFieldStyle {
        thickness: scene.number(node, "thickness")? as f32,
        softness: scene.number(node, "softness")?.max(0.0) as f32,
        outline_width: scene.number(node, "outline_width")?.max(0.0) as f32,
        outline_color: scene.color_value(node, "outline_color")?,
    })
}

struct RectShadow {
    color: Color,
    blur: f64,
    spread: f64,
    offset_x: f64,
    offset_y: f64,
    inner: bool,
}

impl RectShadow {
    fn none() -> Self {
        Self {
            color: Color::rgba8(0, 0, 0, 0),
            blur: 0.0,
            spread: 0.0,
            offset_x: 0.0,
            offset_y: 0.0,
            inner: false,
        }
    }
}

fn rect_shadow(scene: &Scene, node: NodeHandle) -> Result<RectShadow, RenderError> {
    let color = scene.color_value(node, "shadow_color")?;
    if color.alpha <= 0.0 {
        return Ok(RectShadow::none());
    }
    Ok(RectShadow {
        color,
        blur: scene.number(node, "shadow_blur")?.max(0.0),
        spread: scene.number(node, "shadow_spread")?,
        offset_x: scene.number(node, "shadow_offset_x")?,
        offset_y: scene.number(node, "shadow_offset_y")?,
        inner: scene.bool_value(node, "shadow_inner")?,
    })
}
