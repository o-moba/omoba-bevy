//! End-to-end check of the runtime avatar store against a real registry.
//!
//! Needs a registry that carries at least one template approved for Omoba:
//!
//! ```text
//! OMOBA_REGISTRY_URL=http://127.0.0.1:8019 \
//!   cargo test -p omoba-passport --test store_e2e -- --ignored --nocapture
//! ```
use omoba_passport::store::{self, ModelState};

#[test]
#[ignore = "needs OMOBA_REGISTRY_URL pointing at a registry with an Omoba approval"]
fn approved_template_is_listed_installed_and_passes_omoba_verification() {
    let root = std::env::temp_dir().join(format!("omoba-store-e2e-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    store::initialize(root.clone(), true);

    let avatars = shared::store_avatars();
    assert!(!avatars.is_empty(), "registry lists no Omoba-approved template");
    for avatar in avatars {
        let protected = avatar.passport.as_ref().expect("store entries are paid");
        assert_eq!(omoba_passport::protected_slug(protected), avatar.slug);
        assert_eq!(shared::normalize_avatar_slug(Some(&avatar.slug)), Some(avatar.slug.as_str()));
        assert!(store::knows(&avatar.slug));

        store::install_blocking(&avatar.slug).expect("verified install");
        assert_eq!(store::model_state(&avatar.slug), ModelState::Ready);
        let installed = root.join("avatars").join(format!("{}.glb", avatar.slug));
        omoba_passport::verify_local(protected, &installed).expect("Omoba humanoid profile");
        println!(
            "installed {} as {} ({} bytes, thumbnail {:?})",
            avatar.display_name,
            avatar.slug,
            protected.support.rendition.size_bytes,
            avatar.thumbnail
        );
    }
    assert!(!store::take_changed().is_empty());
    let _ = std::fs::remove_dir_all(&root);
}
