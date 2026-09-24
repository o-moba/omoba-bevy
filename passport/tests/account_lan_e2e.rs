//! Opt-in real local account-library check; credentials are never logged.
use omoba_passport::{account, store};
#[test]
#[ignore = "requires a real paired local account and Studio approval"]
fn paired_account_restores_and_installs_its_approved_avatar() {
    let credential = account::AccountCredential::load(std::path::Path::new(
        &std::env::var("OMOBA_TEST_ACCOUNT_FILE").expect("private credential path"),
    ))
    .unwrap()
    .unwrap();
    let session = account::client()
        .unwrap()
        .restore(&credential, &omoba_passport::selector())
        .expect("real account library");
    let id = std::env::var("OMOBA_TEST_AVATAR_ID").expect("uploaded avatar id");
    let item = session
        .items
        .iter()
        .find(|item| item.protected.avatar_id == id)
        .expect("approved avatar in owned library");
    let root = std::env::temp_dir().join(format!("omoba-real-account-{}", std::process::id()));
    store::initialize(root.clone(), true);
    store::install_blocking(&item.slug).expect("verified SDK download and Omoba validation");
    omoba_passport::verify_local(
        &item.protected,
        &root.join("avatars").join(format!("{}.glb", item.slug)),
    )
    .unwrap();
    println!(
        "REAL_STUDIO_AVATAR {}",
        serde_json::json!({"avatar_id":id,"slug":item.slug,"name":item.name,"rendition":item.protected.support.rendition,"account_library":true})
    );
    let _ = std::fs::remove_dir_all(root);
}
