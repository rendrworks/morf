/// Complete layout output keyed by stable node handles.
#[derive(Clone, Debug, Default)]
pub struct Layout {
    geometry: HashMap<NodeHandle, Geometry>,
    implicit: HashMap<NodeHandle, Size>,
}

/// Cached layout geometry used by native transform watchers.
#[derive(Debug, Default)]
pub struct TransformTracker {
    geometry: HashMap<NodeHandle, Geometry>,
}

/// Watches the geometry and transform chain between two scene nodes.
#[derive(Clone, Debug)]
pub struct TransformWatcher {
    a: NodeHandle,
    b: NodeHandle,
    common_parent: Option<NodeHandle>,
    signature: Option<u64>,
}

/// Inputs for an animated parent and anchor change.
pub struct ReparentTransition {
    pub root: NodeHandle,
    pub node: NodeHandle,
    pub new_parent: NodeHandle,
    pub anchors: Option<BTreeMap<String, Value>>,
    pub available: Size,
    pub behavior: Behavior,
}

impl Layout {
    /// Resolves the layout rooted at `root` into the supplied surface area.
    pub fn compute(
        scene: &Scene,
        root: NodeHandle,
        available: Size,
        text: &mut impl TextMeasurer,
    ) -> Result<Self, LayoutError> {
        let mut layout = Self::default();
        layout.measure_implicit(scene, root, text)?;
        layout.geometry.insert(
            root,
            Geometry {
                width: available.width,
                height: available.height,
                ..Geometry::default()
            },
        );
        layout.resolve_children(scene, root)?;
        Ok(layout)
    }

    /// Returns the resolved geometry for a node in the computed tree.
    pub fn geometry(&self, node: NodeHandle) -> Option<Geometry> {
        self.geometry.get(&node).copied()
    }

    /// Iterates over every resolved node geometry.
    pub fn geometries(&self) -> impl Iterator<Item = (NodeHandle, Geometry)> + '_ {
        self.geometry
            .iter()
            .map(|(node, geometry)| (*node, *geometry))
    }

    /// Returns the bottom-up implicit size for a node.
    pub fn implicit_size(&self, node: NodeHandle) -> Option<Size> {
        self.implicit.get(&node).copied()
    }

    /// Reparents a node and animates its resolved position through a shared coordinate space.
    pub fn transition_reparent(
        scene: &mut Scene,
        text: &mut impl TextMeasurer,
        transition: ReparentTransition,
    ) -> Result<Self, LayoutError> {
        let before = Self::compute(scene, transition.root, transition.available, text)?
            .geometry(transition.node)
            .ok_or_else(|| LayoutError::Scene("transition node has no geometry".into()))?;
        scene.assign(transition.node, "transition_x", 0.0)?;
        scene.assign(transition.node, "transition_y", 0.0)?;
        if let Some(anchors) = transition.anchors {
            scene.assign(transition.node, "anchors", Value::Map(anchors))?;
        }
        if scene.parent(transition.node)? != Some(transition.new_parent) {
            scene.reparent(transition.node, Some(transition.new_parent))?;
        }
        let target = Self::compute(scene, transition.root, transition.available, text)?
            .geometry(transition.node)
            .ok_or_else(|| LayoutError::Scene("transition target has no geometry".into()))?;
        scene.animate_from(
            transition.node,
            "transition_x",
            before.x - target.x,
            0.0,
            transition.behavior,
        )?;
        scene.animate_from(
            transition.node,
            "transition_y",
            before.y - target.y,
            0.0,
            transition.behavior,
        )?;
        Self::compute(scene, transition.root, transition.available, text)
    }

    fn measure_implicit(
        &mut self,
        scene: &Scene,
        node: NodeHandle,
        text: &mut impl TextMeasurer,
    ) -> Result<Size, LayoutError> {
        let children = scene.children(node)?;
        let mut child_sizes = Vec::with_capacity(children.len());
        for child in &children {
            let implicit = self.measure_implicit(scene, *child, text)?;
            child_sizes.push(self.requested_size(scene, *child, implicit)?);
        }

        let size = match scene.element(node)? {
            Element::Text => text.measure(
                node,
                scene.string_value(node, "text")?,
                scene.string_value(node, "font_family")?,
                scene.number(node, "font_size")?,
                TextOptions {
                    width: positive(scene.number(node, "width")?),
                    wrap: scene.bool_value(node, "wrap")?,
                    alignment: text_alignment(scene.string_value(node, "horizontal_alignment")?)?,
                    elide: text_elide(scene.string_value(node, "elide")?)?,
                    font_weight: scene.number(node, "font_weight")?,
                    font_source: match scene.string_value(node, "font_source")? {
                        "" => None,
                        source => Some(source.to_owned()),
                    },
                },
            ),
            Element::Image | Element::Icon => {
                let element = scene.element(node)?;
                let source = if element == Element::Image {
                    scene.string_value(node, "source")?
                } else {
                    scene.string_value(node, "name")?
                };
                let theme = (element == Element::Icon)
                    .then(|| scene.string_value(node, "theme"))
                    .transpose()?;
                let natural = text
                    .measure_image(node, element, source, theme)
                    .unwrap_or_default();
                let width = positive(scene.number(node, "source_width")?).unwrap_or(natural.width);
                let height =
                    positive(scene.number(node, "source_height")?).unwrap_or(natural.height);
                Size { width, height }
            }
            Element::Row | Element::RowLayout => Size {
                width: sum_with_spacing(&child_sizes, scene.number(node, "spacing")?, true),
                height: child_sizes
                    .iter()
                    .map(|size| size.height)
                    .fold(0.0, f64::max),
            },
            Element::Column | Element::ColumnLayout => Size {
                width: child_sizes
                    .iter()
                    .map(|size| size.width)
                    .fold(0.0, f64::max),
                height: sum_with_spacing(&child_sizes, scene.number(node, "spacing")?, false),
            },
            Element::Grid | Element::GridLayout => grid_size(
                &child_sizes,
                grid_columns(scene.number(node, "columns")?),
                scene.number(node, "column_spacing")?,
                scene.number(node, "row_spacing")?,
            ),
            Element::Inset => {
                let width = child_sizes.first().map_or(0.0, |size| size.width);
                let height = child_sizes.first().map_or(0.0, |size| size.height);
                Size {
                    width: width
                        + inset_margin(scene, node, "left_margin")?
                        + inset_margin(scene, node, "right_margin")?,
                    height: height
                        + inset_margin(scene, node, "top_margin")?
                        + inset_margin(scene, node, "bottom_margin")?,
                }
            }
            Element::Item
            | Element::Rect
            | Element::ClipRect
            | Element::Shape
            | Element::MouseArea
            | Element::Flickable
            | Element::Loader
            | Element::Timer => {
                let mut bounds = Size::default();
                for (child, size) in children.iter().zip(child_sizes) {
                    bounds.width = bounds.width.max(scene.number(*child, "x")? + size.width);
                    bounds.height = bounds.height.max(scene.number(*child, "y")? + size.height);
                }
                if scene.element(node)? == Element::ClipRect
                    && scene.bool_value(node, "content_inside_border")?
                {
                    let border = scene.number(node, "border_width")?.max(0.0);
                    bounds.width += border * 2.0;
                    bounds.height += border * 2.0;
                }
                bounds
            }
        };
        self.implicit.insert(node, size);
        Ok(size)
    }

    fn requested_size(
        &self,
        scene: &Scene,
        node: NodeHandle,
        implicit: Size,
    ) -> Result<Size, LayoutError> {
        let attached = attached_layout(scene.current(node, "layout")?)?;
        let implicit_width = positive(scene.number(node, "implicit_width")?);
        let implicit_height = positive(scene.number(node, "implicit_height")?);
        let width = positive(scene.number(node, "width")?)
            .or_else(|| layout_number(&attached, "preferred_width"))
            .or(implicit_width)
            .unwrap_or(implicit.width);
        let height = positive(scene.number(node, "height")?)
            .or_else(|| layout_number(&attached, "preferred_height"))
            .or(implicit_height)
            .unwrap_or(implicit.height);
        Ok(Size {
            width: clamp_layout(width, &attached, "minimum_width", "maximum_width"),
            height: clamp_layout(height, &attached, "minimum_height", "maximum_height"),
        })
    }

    fn resolve_children(&mut self, scene: &Scene, parent: NodeHandle) -> Result<(), LayoutError> {
        let mut parent_geometry = self.geometry[&parent];
        let parent_element = scene.element(parent)?;
        if parent_element == Element::ClipRect
            && scene.bool_value(parent, "content_inside_border")?
        {
            let border = scene.number(parent, "border_width")?.max(0.0);
            parent_geometry.x += border;
            parent_geometry.y += border;
            parent_geometry.width = (parent_geometry.width - border * 2.0).max(0.0);
            parent_geometry.height = (parent_geometry.height - border * 2.0).max(0.0);
        }
        let children = scene.children(parent)?;
        let spacing = match parent_element {
            Element::Row | Element::Column | Element::RowLayout | Element::ColumnLayout => {
                scene.number(parent, "spacing")?
            }
            _ => 0.0,
        };
        let mut cursor = 0.0;
        let columns = matches!(parent_element, Element::Grid | Element::GridLayout)
            .then(|| grid_columns(scene.number(parent, "columns").unwrap_or(1.0)))
            .unwrap_or(1);
        let column_spacing = if matches!(parent_element, Element::Grid | Element::GridLayout) {
            scene.number(parent, "column_spacing")?
        } else {
            0.0
        };
        let row_spacing = if matches!(parent_element, Element::Grid | Element::GridLayout) {
            scene.number(parent, "row_spacing")?
        } else {
            0.0
        };
        let mut grid_widths = Vec::new();
        let mut grid_heights = Vec::new();
        if matches!(parent_element, Element::Grid | Element::GridLayout) {
            grid_widths.resize(columns, 0.0_f64);
            grid_heights.resize(children.len().div_ceil(columns), 0.0_f64);
            for (index, child) in children.iter().enumerate() {
                let size = self.requested_size(scene, *child, self.implicit[child])?;
                grid_widths[index % columns] = grid_widths[index % columns].max(size.width);
                grid_heights[index / columns] = grid_heights[index / columns].max(size.height);
            }
        }
        let mut growth = HashMap::new();
        if matches!(parent_element, Element::RowLayout | Element::ColumnLayout) {
            let horizontal = parent_element == Element::RowLayout;
            let mut occupied = spacing * children.len().saturating_sub(1) as f64;
            let mut fillers = Vec::new();
            for child in &children {
                let size = self.requested_size(scene, *child, self.implicit[child])?;
                occupied += if horizontal { size.width } else { size.height };
                let attached = attached_layout(scene.current(*child, "layout")?)?;
                if flag(
                    &attached,
                    if horizontal {
                        "fill_width"
                    } else {
                        "fill_height"
                    },
                ) {
                    fillers.push(*child);
                }
            }
            let available = if horizontal {
                parent_geometry.width
            } else {
                parent_geometry.height
            };
            let share = ((available - occupied).max(0.0) / fillers.len().max(1) as f64).max(0.0);
            for child in fillers {
                growth.insert(child, share);
            }
        }

        for (position, child) in children.into_iter().enumerate() {
            let implicit = self.implicit[&child];
            let size = self.requested_size(scene, child, implicit)?;
            let anchors = anchors(scene.current(child, "anchors")?)?;
            reject_axis_conflict(parent_element, &anchors)?;
            let mut geometry = Geometry {
                x: scene.number(child, "x")?,
                y: scene.number(child, "y")?,
                width: size.width,
                height: size.height,
            };
            let attached = attached_layout(scene.current(child, "layout")?)?;
            if parent_element == Element::Inset {
                let left = inset_margin(scene, parent, "left_margin")?;
                let right = inset_margin(scene, parent, "right_margin")?;
                let top = inset_margin(scene, parent, "top_margin")?;
                let bottom = inset_margin(scene, parent, "bottom_margin")?;
                if scene.bool_value(parent, "resize_child")? {
                    geometry.x = left;
                    geometry.y = top;
                    geometry.width = (parent_geometry.width - left - right).max(0.0);
                    geometry.height = (parent_geometry.height - top - bottom).max(0.0);
                } else {
                    geometry.x =
                        distributed_margin(parent_geometry.width - geometry.width, left, right);
                    geometry.y =
                        distributed_margin(parent_geometry.height - geometry.height, top, bottom);
                }
            } else if parent_element == Element::RowLayout {
                geometry.width += growth.get(&child).copied().unwrap_or(0.0);
                if flag(&attached, "fill_height") {
                    geometry.height = parent_geometry.height;
                }
            } else if parent_element == Element::ColumnLayout {
                geometry.height += growth.get(&child).copied().unwrap_or(0.0);
                if flag(&attached, "fill_width") {
                    geometry.width = parent_geometry.width;
                }
            }
            apply_anchors(parent_geometry, &anchors, &mut geometry);
            match parent_element {
                Element::Row | Element::RowLayout => {
                    geometry.x = cursor;
                    cursor += geometry.width + spacing;
                }
                Element::Column | Element::ColumnLayout => {
                    geometry.y = cursor;
                    cursor += geometry.height + spacing;
                }
                Element::Grid | Element::GridLayout => {
                    let column = position % columns;
                    let row = position / columns;
                    geometry.x =
                        grid_widths[..column].iter().sum::<f64>() + column_spacing * column as f64;
                    geometry.y = grid_heights[..row].iter().sum::<f64>() + row_spacing * row as f64;
                    if parent_element == Element::GridLayout {
                        if flag(&attached, "fill_width") {
                            geometry.width = grid_widths[column];
                        }
                        if flag(&attached, "fill_height") {
                            geometry.height = grid_heights[row];
                        }
                    }
                }
                _ => {}
            }
            geometry.x += parent_geometry.x;
            geometry.y += parent_geometry.y;
            if parent_element == Element::Flickable {
                geometry.x -= scene.number(parent, "content_x")?;
                geometry.y -= scene.number(parent, "content_y")?;
            }
            geometry.x += scene.number(child, "transition_x")?;
            geometry.y += scene.number(child, "transition_y")?;
            self.geometry.insert(child, geometry);
            self.resolve_children(scene, child)?;
        }
        Ok(())
    }
}

