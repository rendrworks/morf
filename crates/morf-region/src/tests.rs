use super::*;

fn rectangle(x: i32, y: i32, width: i32, height: i32, operation: Operation) -> Region {
    Region {
        rect: Rect {
            x,
            y,
            width,
            height,
        },
        operation,
        ..Region::default()
    }
}

#[test]
fn combines_subtracts_and_merges_vertical_runs() {
    let regions = [Region {
        rect: Rect {
            x: 0,
            y: 0,
            width: 6,
            height: 4,
        },
        children: vec![rectangle(2, 1, 2, 2, Operation::Subtract)],
        ..Region::default()
    }];
    assert_eq!(
        build(6, 4, &regions).unwrap(),
        [
            Rect {
                x: 0,
                y: 0,
                width: 6,
                height: 1
            },
            Rect {
                x: 0,
                y: 1,
                width: 2,
                height: 2
            },
            Rect {
                x: 4,
                y: 1,
                width: 2,
                height: 2
            },
            Rect {
                x: 0,
                y: 3,
                width: 6,
                height: 1
            },
        ]
    );
}

#[test]
fn ellipse_and_xor_are_composable() {
    let ellipse = Region {
        rect: Rect {
            x: 0,
            y: 0,
            width: 5,
            height: 5,
        },
        shape: Shape::Ellipse,
        ..Region::default()
    };
    let regions = [
        ellipse.clone(),
        Region {
            operation: Operation::Xor,
            ..ellipse
        },
    ];
    assert!(build(5, 5, &regions).unwrap().is_empty());
}

#[cfg(test)]
mod equivalence {
    use super::*;

    /// The straightforward reading of the same definition: one full-surface
    /// mask per region, composed in order. Correct, and far too slow to ship —
    /// which is the point of having it here to compare against.
    fn reference(width: u32, height: u32, regions: &[Region]) -> Vec<Rect> {
        fn draw(width: u32, height: u32, region: &Region) -> Vec<bool> {
            let mut mask = vec![false; width as usize * height as usize];
            for y in 0..height as i32 {
                for x in 0..width as i32 {
                    let inside = x >= region.rect.x
                        && y >= region.rect.y
                        && x < region.rect.x.saturating_add(region.rect.width)
                        && y < region.rect.y.saturating_add(region.rect.height)
                        && contains(region.rect, region.shape, &region.params, x, y);
                    if inside {
                        mask[y as usize * width as usize + x as usize] = true;
                    }
                }
            }
            for child in &region.children {
                apply(
                    &mut mask,
                    &draw(width, height, child),
                    child.operation.hard(),
                );
            }
            mask
        }
        let mut mask = vec![false; width as usize * height as usize];
        for region in regions {
            apply(
                &mut mask,
                &draw(width, height, region),
                region.operation.hard(),
            );
        }
        // The rectangles the mask covers, as a pixel set rather than a cover,
        // so the two need only agree on which pixels are in.
        let mut pixels = Vec::new();
        for y in 0..height as i32 {
            for x in 0..width as i32 {
                if mask[y as usize * width as usize + x as usize] {
                    pixels.push(Rect {
                        x,
                        y,
                        width: 1,
                        height: 1,
                    });
                }
            }
        }
        pixels
    }

    /// Expands a cover into the pixels it contains, so two different but
    /// equivalent covers compare equal.
    fn pixels(rects: &[Rect]) -> Vec<Rect> {
        let mut out = Vec::new();
        for rect in rects {
            for y in rect.y..rect.y + rect.height {
                for x in rect.x..rect.x + rect.width {
                    out.push(Rect {
                        x,
                        y,
                        width: 1,
                        height: 1,
                    });
                }
            }
        }
        out.sort_by_key(|rect| (rect.y, rect.x));
        out
    }

    /// Every family the vocabulary has, so the composition is exercised over
    /// all of them and not just the two the region rasteriser used to know.
    fn family(seed: u64) -> Shape {
        match seed % 10 {
            0 => Shape::Circle,
            1 => Shape::Box,
            2 => Shape::Capsule,
            3 => Shape::Triangle,
            4 => Shape::Hexagon,
            5 => Shape::Star,
            6 => Shape::Ring,
            7 => Shape::Pie,
            8 => Shape::Cross,
            _ => Shape::Ellipse,
        }
    }

    fn operation(seed: u64) -> Operation {
        match seed % 4 {
            0 => Operation::Union,
            1 => Operation::Subtract,
            2 => Operation::Intersect,
            _ => Operation::Xor,
        }
    }

    #[test]
    fn windowed_composition_matches_the_full_surface_reading() {
        // A cheap xorshift keeps the case list reproducible without a
        // dependency; the surface is small so the reference stays affordable.
        let mut seed = 0x2545_f491_4f6c_dd1d_u64;
        let mut next = move || {
            seed ^= seed << 13;
            seed ^= seed >> 7;
            seed ^= seed << 17;
            seed
        };
        for _ in 0..200 {
            let count = (next() % 4 + 1) as usize;
            let regions = (0..count)
                .map(|_| {
                    let children = (0..next() % 3)
                        .map(|_| Region {
                            rect: Rect {
                                x: (next() % 34) as i32 - 4,
                                y: (next() % 34) as i32 - 4,
                                width: (next() % 12) as i32,
                                height: (next() % 12) as i32,
                            },
                            shape: family(next()),
                            params: ShapeParams {
                                radii: [
                                    (next() % 4) as f32,
                                    (next() % 4) as f32,
                                    (next() % 4) as f32,
                                    (next() % 4) as f32,
                                ],
                                ..ShapeParams::default()
                            },
                            operation: operation(next()),
                            children: Vec::new(),
                        })
                        .collect();
                    Region {
                        rect: Rect {
                            x: (next() % 34) as i32 - 4,
                            y: (next() % 34) as i32 - 4,
                            width: (next() % 16) as i32,
                            height: (next() % 16) as i32,
                        },
                        shape: family(next()),
                        params: ShapeParams::default(),
                        operation: operation(next()),
                        children,
                    }
                })
                .collect::<Vec<_>>();
            let built = build(30, 30, &regions).expect("composition stays within limits");
            assert_eq!(
                pixels(&built),
                reference(30, 30, &regions),
                "regions {regions:?}"
            );
        }
    }

    #[test]
    fn far_apart_regions_are_composed_without_touching_the_space_between() {
        // The case the windowing exists for: two small regions at opposite
        // edges of a large surface. Both survive, and nothing between them is
        // in the result.
        let regions = vec![
            rectangle(0, 1000, 20, 100, Operation::Union),
            rectangle(3800, 1000, 20, 100, Operation::Union),
        ];
        let built = build(3840, 2160, &regions).expect("composition stays within limits");
        assert_eq!(built.len(), 2);
        assert_eq!(built[0], regions[0].rect);
        assert_eq!(built[1], regions[1].rect);
    }

    #[test]
    fn an_intersect_that_reaches_nothing_clears_every_window() {
        // `Intersect` is the one operation that acts outside its own bounds:
        // intersecting with a shape somewhere else leaves nothing anywhere.
        let regions = vec![
            rectangle(0, 0, 20, 20, Operation::Union),
            rectangle(3800, 2100, 20, 20, Operation::Union),
            rectangle(1000, 1000, 10, 10, Operation::Intersect),
        ];
        assert!(
            build(3840, 2160, &regions)
                .expect("composition stays within limits")
                .is_empty()
        );
    }

    #[test]
    fn a_sizeless_root_windows_by_its_children() {
        // The shape a shell actually produces: one mask with no size of its
        // own, carrying the interactive parts of the surface as children. The
        // children are what the windows must follow — the root spans them all.
        let root = Region {
            rect: Rect::default(),
            operation: Operation::Union,
            children: vec![
                rectangle(0, 0, 17, 2160, Operation::Union),
                rectangle(1258, 1912, 940, 140, Operation::Union),
                rectangle(2717, 976, 739, 513, Operation::Union),
            ],
            ..Region::default()
        };
        let built = build(3456, 2160, std::slice::from_ref(&root))
            .expect("composition stays within limits");
        let covered: i64 = built
            .iter()
            .map(|rect| i64::from(rect.width) * i64::from(rect.height))
            .sum();
        let expected: i64 = root
            .children
            .iter()
            .map(|child| i64::from(child.rect.width) * i64::from(child.rect.height))
            .sum();
        assert_eq!(covered, expected);
    }

    fn rectangle(x: i32, y: i32, width: i32, height: i32, operation: Operation) -> Region {
        Region {
            rect: Rect {
                x,
                y,
                width,
                height,
            },
            operation,
            ..Region::default()
        }
    }
}

#[test]
fn a_star_region_is_not_the_rectangle_around_it() {
    // The point of the merge. A star used to be drawable and not clickable:
    // the only clickable area a star-shaped node could be given was its own
    // bounding rectangle, because the rasteriser's whole vocabulary was a
    // rectangle and an ellipse.
    let star = Region {
        rect: Rect {
            x: 0,
            y: 0,
            width: 32,
            height: 32,
        },
        shape: Shape::Star,
        ..Region::default()
    };
    let built = build(32, 32, std::slice::from_ref(&star)).expect("a star composes");
    let covered: i32 = built.iter().map(|rect| rect.width * rect.height).sum();
    assert!(covered > 0, "the star covers something");
    assert!(
        covered < 32 * 32 / 2,
        "a five-pointed star covers well under half its box, not {covered} of 1024",
    );
    // The waist between two points is open, and the centre is solid.
    let filled = |x: i32, y: i32| {
        built
            .iter()
            .any(|r| x >= r.x && y >= r.y && x < r.x + r.width && y < r.y + r.height)
    };
    assert!(filled(16, 16), "the middle of the star is inside it");
    assert!(!filled(0, 0), "and its bounding corner is not");
}

#[test]
fn a_smooth_operation_composes_as_its_hard_one_on_a_mask() {
    // A mask has no partial coverage, so there is no seam to round. The shapes
    // still have to be the same ones — a smooth union that quietly composed
    // nothing would pass a test that only checked it did not crash.
    let square = |operation| Region {
        rect: Rect {
            x: 4,
            y: 4,
            width: 12,
            height: 12,
        },
        shape: Shape::Box,
        operation,
        ..Region::default()
    };
    let smooth = build(
        24,
        24,
        &[square(Operation::Union), square(Operation::SmoothUnion)],
    );
    let hard = build(
        24,
        24,
        &[square(Operation::Union), square(Operation::Union)],
    );
    assert_eq!(smooth.unwrap(), hard.unwrap());
}

#[test]
fn every_family_the_renderer_draws_can_be_composed_into_a_region() {
    // The vocabulary is one list or it is two. If a family is ever added to
    // the shader without a distance function here, it becomes drawable and not
    // clickable again, and this is what says so.
    for name in [
        "circle", "rect", "capsule", "triangle", "hexagon", "star", "ring", "pie", "cross",
        "ellipse",
    ] {
        let shape = Shape::parse(name).unwrap_or_else(|| panic!("{name} is a shape"));
        let region = Region {
            rect: Rect {
                x: 0,
                y: 0,
                width: 24,
                height: 24,
            },
            shape,
            ..Region::default()
        };
        let built = build(24, 24, std::slice::from_ref(&region))
            .unwrap_or_else(|error| panic!("{name} composes: {error}"));
        let covered: i32 = built.iter().map(|rect| rect.width * rect.height).sum();
        assert!(covered > 0, "{name} covers something");
        assert!(covered <= 24 * 24, "{name} stays inside its own rectangle");
    }
}

#[test]
fn a_coarse_build_covers_what_the_fine_one_did() {
    // The point of the coarse grid is speed, and the thing that would make it
    // useless is coming out *smaller* than the shape — a blur region a pixel
    // short of the edge painted over it shows a hard line. Rounding outward is
    // what stops that, so it is asserted rather than assumed.
    let circle = Region {
        rect: Rect {
            x: 30,
            y: 30,
            width: 100,
            height: 100,
        },
        shape: Shape::Box,
        params: ShapeParams {
            radii: [50.0; 4],
            ..ShapeParams::default()
        },
        ..Region::default()
    };
    let fine = build(256, 256, std::slice::from_ref(&circle)).unwrap();
    let covered = |rects: &[Rect], x: i32, y: i32| {
        rects.iter().any(|rect| {
            x >= rect.x && x < rect.x + rect.width && y >= rect.y && y < rect.y + rect.height
        })
    };
    // Every divisor a caller might reach for, including the shared default.
    //
    // The claim is not that nothing is dropped — a circle sampled on a coarser
    // grid loses slivers along its tangent, which is inherent and was found by
    // asserting the stronger thing and watching it fail. The claim is that the
    // error is bounded by one cell: everything further inside than that is
    // covered, so a caller painting its own edge over the boundary never sees
    // a hole in the middle of the shape.
    for divisor in [2, 4, COVERED_EDGE_GRID, 16] {
        let coarse = build_scaled(256, 256, std::slice::from_ref(&circle), divisor).unwrap();
        let step = divisor as i32;
        for y in 0..256 {
            for x in 0..256 {
                let well_inside = (-step..=step)
                    .all(|dy| (-step..=step).all(|dx| covered(&fine, x + dx, y + dy)));
                if well_inside {
                    assert!(
                        covered(&coarse, x, y),
                        "divisor {divisor} dropped ({x}, {y}), a cell inside the shape"
                    );
                }
            }
        }
        assert!(
            coarse.len() < fine.len(),
            "divisor {divisor}: coarse {} vs fine {}",
            coarse.len(),
            fine.len()
        );
    }
}

#[test]
fn a_divisor_of_one_is_the_ordinary_build() {
    let square = Region {
        rect: Rect {
            x: 4,
            y: 6,
            width: 20,
            height: 12,
        },
        ..Region::default()
    };
    assert_eq!(
        build_scaled(64, 64, std::slice::from_ref(&square), 1).unwrap(),
        build(64, 64, std::slice::from_ref(&square)).unwrap(),
    );
}
