mod tests {
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
                        && contains(region.rect, region.shape, x, y);
                    if inside {
                        mask[y as usize * width as usize + x as usize] = true;
                    }
                }
            }
            for child in &region.children {
                apply(&mut mask, &draw(width, height, child), child.operation);
            }
            mask
        }
        let mut mask = vec![false; width as usize * height as usize];
        for region in regions {
            apply(&mut mask, &draw(width, height, region), region.operation);
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

    fn operation(seed: u64) -> Operation {
        match seed % 4 {
            0 => Operation::Combine,
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
                            shape: if next() % 3 == 0 {
                                Shape::Ellipse
                            } else {
                                Shape::Rectangle {
                                    top_left: (next() % 4) as u32,
                                    top_right: (next() % 4) as u32,
                                    bottom_right: (next() % 4) as u32,
                                    bottom_left: (next() % 4) as u32,
                                }
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
                        shape: if next() % 4 == 0 {
                            Shape::Ellipse
                        } else {
                            Shape::default()
                        },
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
            rectangle(0, 1000, 20, 100, Operation::Combine),
            rectangle(3800, 1000, 20, 100, Operation::Combine),
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
            rectangle(0, 0, 20, 20, Operation::Combine),
            rectangle(3800, 2100, 20, 20, Operation::Combine),
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
            shape: Shape::default(),
            operation: Operation::Combine,
            children: vec![
                rectangle(0, 0, 17, 2160, Operation::Combine),
                rectangle(1258, 1912, 940, 140, Operation::Combine),
                rectangle(2717, 976, 739, 513, Operation::Combine),
            ],
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
            shape: Shape::default(),
            operation,
            children: Vec::new(),
        }
    }
}
