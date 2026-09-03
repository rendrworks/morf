use std::collections::BTreeMap;

use crate::{animation::*, types::*};

pub(crate) fn schema(element: Element) -> Vec<PropertySpec> {
    let mut properties = vec![
        number("x", 0.0),
        number("y", 0.0),
        number("width", 0.0),
        number("height", 0.0),
        number("implicit_width", 0.0),
        number("implicit_height", 0.0),
        any("anchors", Value::Map(BTreeMap::new())),
        boolean("visible", true),
        number("opacity", 1.0),
        any("layer", Value::Map(BTreeMap::new())),
        color("color_overlay", Color::rgba8(0, 0, 0, 0)),
        number("z", 0.0),
        boolean("clip", element == Element::ClipRect),
        // Whether the compositor should blur what is behind this node.
        //
        // On every element rather than only the drawn ones, because what it
        // marks is an area of the surface, not a way of painting: an `Item`
        // wrapping a panel is often the honest place to say it.
        boolean("backdrop_blur", false),
        number("rotation", 0.0),
        number("scale", 1.0),
        number("scale_x", 1.0),
        number("scale_y", 1.0),
        number("skew_x", 0.0),
        number("skew_y", 0.0),
        number("translate_x", 0.0),
        number("translate_y", 0.0),
        number("transform_origin_x", 0.5),
        number("transform_origin_y", 0.5),
        number("transition_x", 0.0),
        number("transition_y", 0.0),
        boolean("enabled", true),
        boolean("focus", false),
        any("layout", Value::Map(BTreeMap::new())),
    ];
    match element {
        Element::Item => {}
        Element::Inset => properties.extend([
            number("margin", 0.0),
            number("extra_margin", 0.0),
            any("top_margin", Value::Nil),
            any("right_margin", Value::Nil),
            any("bottom_margin", Value::Nil),
            any("left_margin", Value::Nil),
            boolean("resize_child", true),
        ]),
        Element::Loader => properties.extend([
            boolean("active", true),
            boolean("loading", false),
            boolean("active_async", false),
        ]),
        Element::Timer => {
            properties.extend([
                number("interval", 1_000.0),
                boolean("repeat", false),
                boolean("running", false),
            ]);
        }
        Element::MouseArea => {
            properties.push(any(
                "accepted_buttons",
                Value::List(vec![Value::String("left".to_owned())]),
            ));
        }
        Element::Flickable => {
            properties.extend([
                // Only the offsets. `content_width`/`content_height` were
                // declared beside them and read by nothing — not by layout, not
                // by paint, not by any configuration — so they were two
                // properties a config could set and watch do nothing.
                number("content_x", 0.0),
                number("content_y", 0.0),
            ]);
        }
        Element::Rect | Element::ClipRect => {
            properties.extend([
                color("color", Color::rgba8(255, 255, 255, 255)),
                string("gradient_type", "none"),
                color("gradient_start_color", Color::rgba8(255, 255, 255, 255)),
                color("gradient_end_color", Color::rgba8(0, 0, 0, 255)),
                number("gradient_start_x", 0.0),
                number("gradient_start_y", 0.0),
                number("gradient_end_x", 1.0),
                number("gradient_end_y", 0.0),
                number("gradient_center_x", 0.5),
                number("gradient_center_y", 0.5),
                number("gradient_radius", 0.5),
                number("gradient_angle", 0.0),
                number("radius", 0.0),
                number("top_left_radius", -1.0),
                number("top_right_radius", -1.0),
                number("bottom_right_radius", -1.0),
                number("bottom_left_radius", -1.0),
                number("border_width", 0.0),
                color("border_color", Color::rgba8(0, 0, 0, 0)),
                number("blur", 0.0),
                color("shadow_color", Color::rgba8(0, 0, 0, 0)),
                number("shadow_blur", 0.0),
                number("shadow_spread", 0.0),
                number("shadow_offset_x", 0.0),
                number("shadow_offset_y", 0.0),
                boolean("shadow_inner", false),
            ]);
            if element == Element::ClipRect {
                properties.extend([
                    boolean("content_inside_border", true),
                    boolean("content_under_border", false),
                    boolean("antialiasing", true),
                    boolean("border_pixel_aligned", true),
                ]);
            }
        }
        Element::Text => {
            properties.extend([
                string("text", ""),
                color("color", Color::rgba8(0, 0, 0, 255)),
                number("font_size", 16.0),
                number("font_weight", 400.0),
                string("font_family", "sans-serif"),
                string("font_source", ""),
                boolean("wrap", false),
                string("elide", "none"),
                // Wrapped text stops after this many lines, the last one
                // elided. Zero is no limit.
                number("max_lines", 0.0),
                string("horizontal_alignment", "left"),
                string("vertical_alignment", "top"),
                // Glyphs are distance fields, so the edge is a threshold rather
                // than a set of pixels: these move it, soften it, and read a
                // second one further out as an outline. All ordinary numbers,
                // so all animatable, which is the reason for storing letters
                // this way at all.
                number("thickness", 0.0),
                number("softness", 0.0),
                number("outline_width", 0.0),
                color("outline_color", Color::rgba8(0, 0, 0, 0)),
                // The text this one turns into, and how far along it is.
                //
                // Not a crossfade between two labels: the glyphs are distance
                // fields, so the two are interpolated as fields and thresholded
                // once, and the outline travels from one letter's shape to the
                // other's through shapes that belong to neither. Glyphs pair up
                // by position, and one with nothing opposite it dissolves.
                string("morph_to", ""),
                number("morph_progress", 0.0),
            ]);
        }
        Element::Image => {
            properties.extend([
                string("source", ""),
                string("fill_mode", "stretch"),
                number("source_width", 0.0),
                number("source_height", 0.0),
                boolean("distance_field", false),
                number("distance_field_spread", 8.0),
                // The same four names Text uses, because they are the same
                // four numbers. They were spelled `distance_field_*` here and
                // plainly there, and `weight` even meant a different thing in
                // each — an absolute threshold on one side and a signed offset
                // on the other.
                number("thickness", 0.0),
                number("softness", 0.0),
                number("outline_width", 0.0),
                color("outline_color", Color::rgba8(0, 0, 0, 0)),
            ]);
        }
        Element::Icon => {
            properties.extend([
                string("name", ""),
                string("theme", "hicolor"),
                string("fill_mode", "stretch"),
                number("source_width", 0.0),
                number("source_height", 0.0),
                boolean("distance_field", false),
                number("distance_field_spread", 8.0),
                number("thickness", 0.0),
                number("softness", 0.0),
                number("outline_width", 0.0),
                color("outline_color", Color::rgba8(0, 0, 0, 0)),
            ]);
        }
        Element::Sdf => {
            properties.extend([
                color("fill_color", Color::rgba8(255, 255, 255, 255)),
                color("stroke_color", Color::rgba8(0, 0, 0, 0)),
                number("stroke_width", 0.0),
                // Extra edge softness in logical pixels, on top of the
                // derivative-based antialiasing the shader always applies. A
                // field is resolution independent, so this is the one knob that
                // turns a crisp edge into a glow.
                number("softness", 0.0),
                // The seam radius every absorbed layer uses unless it names its
                // own. A field with a blend fuses what it contains; a field
                // without one composes the same shapes with hard edges.
                number("blend", 0.0),
                // One position along the morph for the whole composition. A
                // compound shape — a disc with a ring and a notch, say — is
                // several layers that have to move together, and keeping that
                // many numbers in step by hand is how a configuration acquires
                // a frame runtime. Driving them from here makes the compound
                // one animatable property.
                number("morph_progress", 0.0),
                // Everything below belonged to a rectangle alone, because a
                // rectangle had its own pipeline and a composed shape did not.
                // One pipeline draws both now, so a star can carry a gradient
                // and a shadow like anything else.
                string("gradient_type", "none"),
                color("gradient_start_color", Color::rgba8(255, 255, 255, 255)),
                color("gradient_end_color", Color::rgba8(0, 0, 0, 255)),
                number("gradient_start_x", 0.0),
                number("gradient_start_y", 0.0),
                number("gradient_end_x", 1.0),
                number("gradient_end_y", 0.0),
                number("gradient_center_x", 0.5),
                number("gradient_center_y", 0.5),
                number("gradient_radius", 0.5),
                number("gradient_angle", 0.0),
                color("shadow_color", Color::rgba8(0, 0, 0, 0)),
                number("shadow_blur", 0.0),
                number("shadow_spread", 0.0),
                number("shadow_offset_x", 0.0),
                number("shadow_offset_y", 0.0),
                boolean("shadow_inner", false),
                // Where the stroke sits against the edge: inside, centred or
                // outside. A rectangle border has always been inside and a
                // field stroke centred; they are one outline now, so both are
                // sayable on either.
                string("stroke_alignment", "centre"),
            ]);
        }
        Element::SdfShape => {
            properties.extend([
                string("shape", "circle"),
                // A letter, as a shape in the composition rather than as text
                // drawn beside it. Naming one makes this layer that letter's
                // outline, which then unions, subtracts and morphs with a
                // circle by the same arithmetic a circle does — so a numeral
                // cut out of a disc is a subtraction, and the disc becoming a
                // square while the numeral becomes another is one animation.
                //
                // `glyph_morph_to` names the letter it turns into, walked at
                // `morph_progress` alongside whatever the shapes are doing.
                string("glyph", ""),
                string("glyph_morph_to", ""),
                // A drawing, on exactly the same terms. An SVG is a set of
                // closed curves and so is a letter, so naming a file here makes
                // this layer that drawing's outline — which then unions,
                // subtracts and morphs like every other shape, including into a
                // letter or a circle. Nothing is rasterised on the way: a
                // picture of a shape has pixels rather than points, and there is
                // nothing in a picture to walk onto anything else.
                //
                // `source_morph_to` names the drawing it turns into, walked at
                // `morph_progress` beside whatever the shapes are doing.
                string("source", ""),
                string("source_morph_to", ""),
                // Which face the letter is cut from, and which the letter it
                // turns into is cut from. Empty means the same face, which is
                // the ordinary case; naming a second one morphs across faces,
                // since matching two outlines is geometry and does not care
                // which font either of them came out of.
                string("font_family", "sans-serif"),
                string("font_family_morph_to", ""),
                // The layer's own fill. Fully transparent means "take the
                // field's", which is what keeps a single-colour composition
                // from having to repeat itself on every layer.
                color("fill_color", Color::rgba8(0, 0, 0, 0)),
                // The field this layer becomes at `morph_progress` of one.
                // Interpolating two distance fields passes through shapes that
                // neither end describes, and survives a change of topology —
                // one blob splitting into two — which interpolating outlines
                // cannot do at all.
                string("morph_to", ""),
                // Negative means "follow the field's", so a layer joins the
                // compound morph by saying nothing and leaves it by naming its
                // own position.
                number("morph_progress", -1.0),
                string("operation", "union"),
                // How far either side of the seam a smooth operation blends.
                // Zero is the hard boolean; animating it is what makes two
                // shapes merge and part like liquid.
                number("blend", 0.0),
                number("radius", 0.0),
                // Per-corner overrides, as a Rect carries them: negative means
                // "use the uniform radius". A field box keeps all four, so a
                // rect absorbed into a composition keeps its own shape.
                number("top_left_radius", -1.0),
                number("top_right_radius", -1.0),
                number("bottom_right_radius", -1.0),
                number("bottom_left_radius", -1.0),
                number("points", 5.0),
                number("inner_radius", 0.5),
                number("thickness", 0.0),
                number("angle", 90.0),
            ]);
        }
        Element::Row | Element::Column | Element::RowLayout | Element::ColumnLayout => {
            properties.extend([
                number("spacing", 0.0),
                // Where children sit across the axis the positioner packs
                // along: `start`, `center`, `end`, or `stretch` to the
                // positioner's own extent. A child's `layout.alignment`
                // overrides it for that child.
                string("alignment", "start"),
            ]);
        }
        Element::Grid | Element::GridLayout => {
            properties.extend([
                number("columns", 1.0),
                number("row_spacing", 0.0),
                number("column_spacing", 0.0),
            ]);
        }
    }
    properties
}

pub(crate) fn any(name: &'static str, default: Value) -> PropertySpec {
    PropertySpec {
        name,
        kind: PropertyType::Any,
        default,
    }
}

pub(crate) fn boolean(name: &'static str, default: bool) -> PropertySpec {
    PropertySpec {
        name,
        kind: PropertyType::Bool,
        default: Value::Bool(default),
    }
}

pub(crate) fn number(name: &'static str, default: f64) -> PropertySpec {
    PropertySpec {
        name,
        kind: PropertyType::Number,
        default: Value::Number(default),
    }
}

pub(crate) fn string(name: &'static str, default: &str) -> PropertySpec {
    PropertySpec {
        name,
        kind: PropertyType::String,
        default: Value::String(default.to_owned()),
    }
}

pub(crate) fn color(name: &'static str, default: Color) -> PropertySpec {
    PropertySpec {
        name,
        kind: PropertyType::Color,
        default: Value::Color(default),
    }
}

pub(crate) fn coerce(
    element: Element,
    property: &str,
    kind: PropertyType,
    value: Value,
) -> Result<Value, SceneError> {
    let converted = match (kind, value) {
        (PropertyType::Any, value) => Some(value),
        (PropertyType::Bool, Value::Bool(value)) => Some(Value::Bool(value)),
        (PropertyType::Number, Value::Number(value)) if value.is_finite() => {
            Some(Value::Number(value))
        }
        (PropertyType::String, Value::String(value)) => Some(Value::String(value)),
        (PropertyType::Color, Value::Color(value)) => Some(Value::Color(value)),
        (PropertyType::Color, Value::String(value)) => Color::parse(&value).map(Value::Color),
        _ => None,
    };
    converted.ok_or_else(|| SceneError::InvalidPropertyType {
        element: element.name(),
        property: property.to_owned(),
        expected: match kind {
            PropertyType::Any => "value",
            PropertyType::Bool => "boolean",
            PropertyType::Number => "finite number",
            PropertyType::String => "string",
            PropertyType::Color => "color",
        },
    })
}
