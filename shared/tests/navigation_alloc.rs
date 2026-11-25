//! A route plan reuses per-thread search buffers: once warm, it allocates
//! only the route it returns (docs/plans/launch-readiness.md, LR-10).
use std::alloc::{GlobalAlloc, Layout, System};
use std::cell::Cell;

use shared::navigation::{Disc, world_navigation};

struct Counting;

thread_local! {
    static ALLOCATIONS: Cell<usize> = const { Cell::new(0) };
}

fn count() {
    let _ = ALLOCATIONS.try_with(|count| count.set(count.get() + 1));
}

// SAFETY: every call is forwarded unchanged to the system allocator.
unsafe impl GlobalAlloc for Counting {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        count();
        unsafe { System.alloc(layout) }
    }

    unsafe fn alloc_zeroed(&self, layout: Layout) -> *mut u8 {
        count();
        unsafe { System.alloc_zeroed(layout) }
    }

    unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
        count();
        unsafe { System.realloc(ptr, layout, new_size) }
    }

    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        unsafe { System.dealloc(ptr, layout) }
    }
}

#[global_allocator]
static GLOBAL: Counting = Counting;

fn allocations() -> usize {
    ALLOCATIONS.with(Cell::get)
}

#[test]
fn a_warm_route_plan_allocates_only_the_route_it_returns() {
    let map = world_navigation();
    let scale = shared::map::WORLD_SCALE;
    let discs = [
        Disc {
            center: [20.0, 0.0],
            radius: 3.2,
        },
        Disc {
            center: [10.0, 10.0],
            radius: 6.0,
        },
    ];
    // A detour around a trunk, an occupied destination (the whole
    // reachable component), and a long order across a live disc.
    let orders = [
        (
            [66.641_04 * scale, 11.667_086 * scale],
            [76.641_04 * scale, 11.667_086 * scale],
        ),
        ([0.0, 0.0], discs[0].center),
        ([-60.0, -60.0], [60.0, 60.0]),
        ([-20.0, 30.0], [30.0, -5.0]),
    ];
    for (index, (start, destination)) in orders.into_iter().enumerate() {
        // None of them is a straight walk, so each one searches the grid.
        assert!(
            index == 1 || !map.segment_clear_with_discs(start, destination, &discs),
            "order {index} is a straight walk"
        );
        assert!(map.plan_route(start, destination, &discs).is_some());
    }
    for (start, destination) in orders {
        let before = allocations();
        let route = map.plan_route(start, destination, &discs);
        let spent = allocations() - before;
        assert!(route.is_some_and(|route| !route.is_empty()));
        assert_eq!(
            spent, 1,
            "{start:?} -> {destination:?}: {spent} allocations"
        );
    }
}
