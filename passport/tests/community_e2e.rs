//! End-to-end check of free Ekza Studio avatars against a real registry.
//!
//! Needs a registry whose `/v2/avatars` lists at least one avatar published through
//! Ekza Studio, built for `desktop / humanoid-glb-v1` and approved by an Omoba owner:
//!
//! ```text
//! OMOBA_REGISTRY_URL=http://127.0.0.1:8137 \
//!   cargo test -p omoba-passport --test community_e2e -- --ignored --nocapture
//! ```
use omoba_passport::{community, store};

#[test]
#[ignore = "needs OMOBA_REGISTRY_URL pointing at a registry with a free Omoba approval"]
fn free_studio_avatar_is_listed_for_the_server_and_installs_on_the_client() {
    // What the game server does before admitting a ticketless join.
    let free = community::fetch_free(&community::registry_url()).expect("registry read");
    assert!(!free.is_empty(), "registry lists no free avatar approved for Omoba");
    assert!(free.iter().all(|item| item.free));

    // What a game client does: the same avatars arrive through the SDK store.
    let root = std::env::temp_dir().join(format!("omoba-community-e2e-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    store::initialize(root.clone(), true);
    for item in &free {
        assert!(item.protected.avatar_id.starts_with("ekza:avatar:"), "{}", item.protected.avatar_id);
        assert_eq!(omoba_passport::protected_slug(&item.protected), item.slug);
        let entry = shared::avatar_definition(&item.slug).expect("registered by the store");
        assert!(entry.free, "the client must know it needs no wallet");
        assert_eq!(entry.passport.as_ref(), Some(&item.protected));

        store::install_blocking(&item.slug).expect("verified install");
        let installed = root.join("avatars").join(format!("{}.glb", item.slug));
        // The game's own Rust profile check on the bytes the registry's builder made.
        omoba_passport::verify_local(&item.protected, &installed).expect("Omoba humanoid profile");
        println!(
            "free avatar {} by {:?}: {} ({} bytes)",
            item.name, item.author, item.slug, item.protected.support.rendition.size_bytes
        );
    }
    let _ = std::fs::remove_dir_all(&root);
}
