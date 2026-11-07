fn main() {
 let url = std::env::var("OMOBA_DATABASE_URL").unwrap();
 let rt = tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap();
 rt.block_on(async {
  use omoba_career_store::career_store::{CareerStore, ready::{Action,Seat}};
  let store = CareerStore::connect(&url).await.unwrap();
  let profile = store.authenticate(&"a".repeat(64), "RetentionFixture").await.unwrap();
  let id = "b".repeat(32);
  store.ready_reserve(&id, shared::match_service::MatchPreference::Casual,
   vec![Seat { profile_id: profile.profile_id, session_id: "retention-fixture".into() }]).await.unwrap();
  store.ready_update(&id, Action::Cancel).await.unwrap();
 });
 println!("Prepared real closed consent receipt in the isolated fixture.");
}
