use super::*;

fn bounds(half: f32) -> Bounds {
    Bounds {
        min: [-half; 2],
        max: [half; 2],
    }
}

fn rectangle(id: &str, min: Point, max: Point) -> Obstacle {
    Obstacle {
        id: id.into(),
        kind: "test_solid".into(),
        vertices: vec![min, [max[0], min[1]], max, [min[0], max[1]]],
    }
}

fn assert_safe(map: &NavigationMap, start: Point, route: &[Point], dynamic: &[Disc]) {
    assert!(route.len() <= MAX_ROUTE_WAYPOINTS);
    let mut previous = start;
    for &point in route {
        assert!(
            map.planning_segment_clear(previous, point, dynamic, false),
            "unsafe segment {previous:?} -> {point:?}"
        );
        let steps = (distance_squared(previous, point).sqrt() / 0.05)
            .ceil()
            .max(1.0) as usize;
        for step in 0..=steps {
            let sample = add(
                previous,
                scale(sub(point, previous), step as f32 / steps as f32),
            );
            assert!(map.point_clear(sample));
            for disc in dynamic {
                assert!(
                    distance_squared(sample, disc.center) >= (disc.radius + HERO_RADIUS).powi(2)
                );
            }
        }
        previous = point;
    }
}

#[test]
fn open_orders_clamp_bounds_preserve_arrival_and_reject_invalid_data() {
    let map = NavigationMap::new(bounds(10.0), vec![]).unwrap();
    assert_eq!(
        map.plan_route([-5.0, 0.0], [30.0, 4.0], &[]),
        Some(vec![[10.0, 4.0]])
    );
    assert_eq!(map.plan_route([0.0, 0.0], [0.0, 0.0], &[]), Some(vec![]));
    assert!(map.plan_route([f32::NAN, 0.0], [0.0, 0.0], &[]).is_none());
    assert!(
        map.plan_route([0.0, 0.0], [f32::INFINITY, 0.0], &[])
            .is_none()
    );
    assert!(map.plan_route([-11.0, 0.0], [0.0, 0.0], &[]).is_none());
    for radius in [-1.0, f32::NAN, f32::INFINITY] {
        assert!(
            map.plan_route(
                [0.0, 0.0],
                [1.0, 0.0],
                &[Disc {
                    center: [5.0; 2],
                    radius
                }]
            )
            .is_none()
        );
    }
    assert!(
        map.plan_route(
            [0.0, 0.0],
            [1.0, 0.0],
            &vec![
                Disc {
                    center: [5.0; 2],
                    radius: 1.3
                };
                MAX_DYNAMIC_DISCS + 1
            ]
        )
        .is_none()
    );
    assert!(NavigationMap::new(bounds(0.0), vec![]).is_err());
    assert!(NavigationMap::new(bounds(4096.0), vec![]).is_err());
    assert!(
        NavigationMap::from_json(
            r#"{"format_version":2,"bounds":{"min":[-1,-1],"max":[1,1]},"obstacles":[]}"#
        )
        .is_err()
    );
    let duplicate = rectangle("duplicate", [-1.0; 2], [1.0; 2]);
    assert!(NavigationMap::new(bounds(10.0), vec![duplicate.clone(), duplicate]).is_err());
    let mut concave = rectangle("concave", [-2.0; 2], [2.0; 2]);
    concave.vertices.insert(2, [0.0, 0.0]);
    assert!(NavigationMap::new(bounds(10.0), vec![concave]).is_err());
}

#[test]
fn forest_gap_uses_hero_clearance_and_detours_when_the_gap_is_too_small() {
    for (gap, direct) in [(1.6, true), (1.0, false)] {
        let map = NavigationMap::new(
            bounds(10.0),
            vec![
                rectangle("south_trees", [-3.0, -4.0], [3.0, -gap / 2.0]),
                rectangle("north_trees", [-3.0, gap / 2.0], [3.0, 4.0]),
            ],
        )
        .unwrap();
        let start = [-8.0, 0.0];
        let destination = [8.0, 0.0];
        let route = map.plan_route(start, destination, &[]).unwrap();
        assert_eq!(route.len() == 1, direct);
        assert_eq!(route.last(), Some(&destination));
        assert_safe(&map, start, &route, &[]);
    }
}

#[test]
fn concave_arrangement_routes_out_of_the_open_side_before_reaching_the_back() {
    let map = NavigationMap::new(
        bounds(12.0),
        vec![
            rectangle("left", [-5.0, -5.0], [-3.0, 5.0]),
            rectangle("right", [3.0, -5.0], [5.0, 5.0]),
            rectangle("back", [-5.0, -5.0], [5.0, -3.0]),
        ],
    )
    .unwrap();
    let start = [0.0, -1.0];
    let destination = [0.0, -9.0];
    let route = map.plan_route(start, destination, &[]).unwrap();
    assert!(route.iter().any(|point| point[1] > 5.5));
    assert_eq!(route.last(), Some(&destination));
    assert_safe(&map, start, &route, &[]);
}

#[test]
fn diagonal_contact_cannot_cut_the_shared_obstacle_corner() {
    let map = NavigationMap::new(
        bounds(8.0),
        vec![
            rectangle("southwest", [-2.0, -2.0], [0.0, 0.0]),
            rectangle("northeast", [0.0, 0.0], [2.0, 2.0]),
        ],
    )
    .unwrap();
    let start = [-4.0, 4.0];
    let destination = [4.0, -4.0];
    assert!(!map.segment_clear(start, destination));
    let route = map.plan_route(start, destination, &[]).unwrap();
    assert!(route.len() > 1);
    assert_safe(&map, start, &route, &[]);
}

#[test]
fn authoritative_sweep_stops_large_steps_and_allows_only_outward_overlap_recovery() {
    let map =
        NavigationMap::new(bounds(10.0), vec![rectangle("trunk", [-1.0; 2], [1.0; 2])]).unwrap();
    let clipped = map.clip_movement([-8.0, 0.0], [8.0, 0.0]);
    assert!((clipped[0] + 1.501).abs() < 0.002);
    assert_eq!(clipped[1], 0.0);
    assert!(map.segment_clear([-8.0, 0.0], clipped));
    assert_eq!(map.clip_movement([1.2, 0.0], [-8.0, 0.0]), [1.2, 0.0]);
    assert_eq!(map.clip_movement([1.2, 0.0], [1.3, 0.0]), [1.3, 0.0]);
    assert_eq!(map.clip_movement([-8.0, 0.0], [f32::NAN, 0.0]), [-8.0, 0.0]);
    let route = map.plan_route([1.2, 0.0], [-8.0, 0.0], &[]).unwrap();
    assert!(route[0][0] >= 1.2);
    assert!(map.point_clear(route[0]));
    assert_safe(&map, route[0], &route[1..], &[]);
}

#[test]
fn occupied_endpoints_project_to_reachable_boundary_and_clear_unreachable_targets_fail() {
    let map =
        NavigationMap::new(bounds(10.0), vec![rectangle("trunk", [-1.0; 2], [1.0; 2])]).unwrap();
    let start = [-8.0, 0.0];
    let route = map.plan_route(start, [0.0, 0.0], &[]).unwrap();
    let endpoint = *route.last().unwrap();
    assert!((endpoint[0] + 1.651).abs() < 0.01);
    assert!(endpoint[1].abs() < 0.01);
    assert_safe(&map, start, &route, &[]);
    let divided = NavigationMap::new(
        bounds(5.0),
        vec![rectangle("wall", [-0.1, -5.0], [0.1, 5.0])],
    )
    .unwrap();
    assert!(divided.plan_route([-4.0, 0.0], [4.0, 0.0], &[]).is_none());
    let covered =
        NavigationMap::new(bounds(1.0), vec![rectangle("covered", [-1.0; 2], [1.0; 2])]).unwrap();
    assert!(covered.plan_route([0.0, 0.0], [1.0, 0.0], &[]).is_none());
}

#[test]
fn translation_and_structure_query_order_preserve_deterministic_routes() {
    let mut baseline = None;
    for offset in [[0.0, 0.0], [150.25, -150.25]] {
        let map = NavigationMap::new(
            Bounds {
                min: add([-20.0; 2], offset),
                max: add([20.0; 2], offset),
            },
            vec![rectangle(
                "tree",
                add([-1.0, -3.0], offset),
                add([1.0, 1.0], offset),
            )],
        )
        .unwrap();
        let start = add([-12.0, 0.0], offset);
        let destination = add([12.0, 0.0], offset);
        let mut discs = [
            Disc {
                center: add([-5.0, 0.0], offset),
                radius: 1.3,
            },
            Disc {
                center: add([5.0, 0.0], offset),
                radius: 3.2,
            },
        ];
        let route = map.plan_route(start, destination, &discs).unwrap();
        assert_safe(&map, start, &route, &discs);
        discs.reverse();
        assert_eq!(
            map.plan_route(start, destination, &discs),
            Some(route.clone())
        );
        let relative: Vec<_> = route.iter().map(|&point| sub(point, offset)).collect();
        if let Some(baseline) = &baseline {
            assert_eq!(&relative, baseline);
        } else {
            baseline = Some(relative);
        }
    }
}

#[test]
fn current_world_cache_keeps_lane_access_and_routes_around_a_shipped_trunk() {
    let map = world_navigation();
    assert!(std::ptr::eq(map, world_navigation()));
    assert!(map.obstacles().len() >= 200);
    let start = [-74.6, -74.6];
    let destination = [74.6, 74.6];
    assert!(map.segment_clear(start, destination));
    assert_safe(
        map,
        start,
        &map.plan_route(start, destination, &[]).unwrap(),
        &[],
    );
    let start = [66.641_04, 11.667_086];
    let destination = [76.641_04, 11.667_086];
    assert!(map.point_clear(start) && map.point_clear(destination));
    assert!(!map.segment_clear(start, destination));
    let route = map.plan_route(start, destination, &[]).unwrap();
    assert!(route.len() > 1);
    assert_eq!(route.last(), Some(&destination));
    assert_safe(map, start, &route, &[]);
}
