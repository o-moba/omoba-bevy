use shared::prematch::Role;
#[derive(Default)]
pub struct DraftState {
    pub capable: bool,
    pub role: Role,
    pub locked: bool,
    pub loaded: bool,
    pub request_id: u64,
    pub acknowledged_request_id: u64,
    pub error: Option<String>,
}
