//! Exercise forest authority through the real decoded-packet handler.
use super::*;

#[test]
fn transform_packets_cannot_tunnel_through_trees_but_a_legal_route_arrives() {
    let map = shared::navigation::world_navigation();
    let (start, end, center) = map
        .obstacles()
        .iter()
        .filter(|o| o.kind == "tree_trunk")
        .find_map(|o| {
            let center = o
                .vertices
                .iter()
                .fold([0.0, 0.0], |a, p| [a[0] + p[0], a[1] + p[1]])
                .map(|x| x / o.vertices.len() as f32);
            let start = [center[0] - 4.0, center[1]];
            let end = [center[0] + 4.0, center[1]];
            (map.point_clear(start)
                && map.point_clear(end)
                && !map.segment_clear(start, end)
                && map.plan_route(start, end, &[]).is_some())
            .then_some((start, end, center))
        })
        .unwrap();
    let socket = UdpSocket::bind("127.0.0.1:0").unwrap();
    socket.set_nonblocking(true).unwrap();
    let mut runtime = ServerRuntime::new(socket, MatchConfig::dev());
    let address: SocketAddr = "127.0.0.1:55991".parse().unwrap();
    let mut now = Instant::now();
    runtime.handle_packet(
        address,
        ClientPacket::Join {
            team: Team::Green,
            character: CharacterChoice::Cube,
            hero_class: HeroClass::Warrior,
            avatar: None,
            sprite_character: None,
            session_id: Some("forest-authority-test".into()),
            passport_ticket: None,
        },
        now,
    );
    let player = runtime.players.get_mut(&address).unwrap();
    player.state.x = start[0];
    player.state.z = start[1];
    player.speed_mult = 100.0; // Even a legal large debug-speed step cannot tunnel.
    for _ in 0..12 {
        now += Duration::from_millis(100);
        runtime.handle_packet(
            address,
            ClientPacket::Transform {
                x: end[0],
                y: PLAYER_GROUND_Y,
                z: end[1],
                yaw: 0.0,
            },
            now,
        );
        let p = &runtime.players[&address].state;
        assert!(
            p.x < center[0] - 0.5,
            "direct packets passed through the trunk"
        );
        assert!(map.point_clear([p.x, p.z]));
    }
    let player = runtime.players.get_mut(&address).unwrap();
    player.state.x = start[0];
    player.state.z = start[1];
    player.speed_mult = 1.0;
    let route = map.plan_route(start, end, &[]).unwrap();
    let mut steps = 0;
    for waypoint in route {
        loop {
            let p = &runtime.players[&address].state;
            let from = [p.x, p.z];
            let dx = waypoint[0] - p.x;
            let dz = waypoint[1] - p.z;
            let distance = dx.hypot(dz);
            if distance < 0.005 {
                break;
            }
            let scale = (PLAYER_SPEED * 0.1 / distance).min(1.0);
            now += Duration::from_millis(100);
            runtime.handle_packet(
                address,
                ClientPacket::Transform {
                    x: from[0] + dx * scale,
                    y: PLAYER_GROUND_Y,
                    z: from[1] + dz * scale,
                    yaw: 0.0,
                },
                now,
            );
            let p = &runtime.players[&address].state;
            assert!(map.segment_clear(from, [p.x, p.z]));
            assert!((p.x - from[0]).hypot(p.z - from[1]) <= 0.501);
            steps += 1;
            assert!(steps < 160, "legal forest route stopped making progress");
        }
    }
    let p = &runtime.players[&address].state;
    assert!((p.x - end[0]).hypot(p.z - end[1]) < 0.01);
}
