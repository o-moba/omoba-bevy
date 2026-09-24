//! One handler per `ClientPacket` variant, grouped by concern.
//!
//! `runtime::dispatch::handle_packet_authorized` runs the ordered admission
//! pre-checks and then calls exactly one of these. Every handler returns
//! `ControlFlow<()>`: `Continue` runs the dispatcher's post-command tail
//! (endpoint touch, sandbox roster, practice bots, prematch, round start,
//! career registration), `Break` returns before it, exactly where the old
//! single `match` returned early.

mod combat;
mod join;
mod movement;
mod session;
mod shop;
mod tools;
mod utility;
