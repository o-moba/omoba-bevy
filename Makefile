.DEFAULT_GOAL := help

.PHONY: help server server-dev practice practice-server game game2d start start-release play play-bots bots stop restart verify-task-12 verify-gameplay iphone-check iphone

# ---------------------------------------------------------------------------
# Match modes (TASK-22)
#
#   release (default) - production-like: the match forms to a full 5v5
#                       roster with server-assigned balanced teams before it
#                       starts. A solo player waits in "Searching for match".
#   dev               - local development ONLY: the first join starts the
#                       match immediately and the client-chosen team is
#                       honored. Never ship this mode.
#
#   practice          - solo start; server bots fill vacant seats and late
#                       humans replace them. Local results, no ranked credit.
#
# Env vars: OMOBA_MATCH_MODE=release|dev|practice, OMOBA_TEAM_SIZE=<players per team,
# default 5>, SERVER_ADDR (server bind), GAME_SERVER_ADDR (client target),
# OMOBA_AUTOJOIN=<class>:<avatar|->:<team> (client joins without UI).
# ---------------------------------------------------------------------------

GAME_SERVER_ADDR ?= 127.0.0.1:4000
LOCAL_SERVER_ADDR ?= 127.0.0.1:4000
IPHONE_OUTPUT ?= builds/iphone

# Bare make is discovery only; game and toolchain processes start explicitly.
help: ## Show commands, common overrides and documentation
	@awk 'BEGIN { FS = ":.*## "; print "Open Moba - play, create, build together\n" } /^[a-zA-Z0-9_-]+:.*## / { printf "  make %-19s %s\n", $$1, $$2 } END { print "\nOverrides:"; print "  make practice LOCAL_SERVER_ADDR=127.0.0.1:4010"; print "  make practice-server LOCAL_SERVER_ADDR=0.0.0.0:4000"; print "  make game GAME_SERVER_ADDR=192.168.1.10:4000"; print "  make bots BOTS=4 BOTS_SERVER=127.0.0.1:4000"; print "  make iphone IPHONE_OUTPUT=builds/iphone"; print "\nDocs: README.md, RUNBOOK.md, mobile/ios/README.md"; print "TestFlight: mobile/ios/TESTFLIGHT.md (separate signing/upload steps)"; print "SDK: https://github.com/ekza-space/ekza-bevy-sdk" }' $(MAKEFILE_LIST)

# Physical iOS package; install the Rust iOS target and select Xcode first.
# Optional signing with existing credentials is documented in mobile/ios/README.md.
iphone-check: ## Check physical iPhone build prerequisites (Xcode + Rust target)
	python3 mobile/ios/build_device.py --check

iphone: ## Build an unsigned physical iPhone package into builds/iphone
	python3 mobile/ios/build_device.py --output "$(IPHONE_OUTPUT)"

# Run the game server in RELEASE match mode (matches form to 5v5 before starting).
server: ## Game server with full-roster matchmaking by default
	cargo run -p server

# Run the game server in DEV match mode (first join starts the match immediately).
server-dev: ## Development server; first player starts the match
	OMOBA_MATCH_MODE=dev cargo run -p server

# Native bot practice: build current sources, supervise this server and client.
# No external harness bots; closing the client stops only this session's server.
practice: ## Play locally with native server bots; no database or wallet needed
	python3 scripts/play_local.py --mode practice --bind "$(LOCAL_SERVER_ADDR)"

# Server-only practice, suitable for late joining testers. Ctrl+C stops this host.
practice-server: ## Host native bot practice; humans replace bots as they join
	SERVER_ADDR="$(LOCAL_SERVER_ADDR)" OMOBA_MATCH_MODE=practice cargo run -p server --locked

# Run a single game client (env: GAME_SERVER_ADDR, default 127.0.0.1:4000)
game: ## Desktop client; set GAME_SERVER_ADDR for another host
	GAME_SERVER_ADDR=$(GAME_SERVER_ADDR) cargo run -p client

# Run one client in the genuine orthographic XY renderer.  The inline mode
# assignment intentionally overrides any conflicting caller environment.
# GAME_SERVER_ADDR is inherited exactly like `make game`.
game2d: ## Desktop client with the orthographic 2D renderer
	GAME_SERVER_ADDR=$(GAME_SERVER_ADDR) OMOBA_PLAYER_VISUAL_MODE=sprite2d cargo run -p client

# DEV quick-start: dev-mode server and two clients for single-machine testing.
# Instant match start, no matchmaking gate. Server and first client run in the
# background; second client runs in the foreground. Clean up with `make stop`.
start: ## Legacy dev server + two clients; use stop afterwards
	OMOBA_MATCH_MODE=dev cargo run -p server &
	cargo run -p client &
	cargo run -p client

# RELEASE-like start: release-mode server in the background plus one client in
# the foreground. The client waits in matchmaking until the roster is full —
# fill the remaining seats with `make bots` from another terminal.
start-release: ## Legacy full-roster server + one client; fill seats with bots
	cargo run -p server &
	cargo run -p client

# Build current locked sources once, then start a local 3D release 5v5 with
# nine bots. Choose a hero and Join. Closing the client or pressing Ctrl+C
# stops this session's server and bots. Override LOCAL_SERVER_ADDR if needed.
play: ## Supervised 5v5 matchmaking demo with nine external harness bots
	python3 scripts/play_local.py --bind "$(LOCAL_SERVER_ADDR)"

play-bots: play ## Alias for play

# Fill bots for a running server: joins BOTS players (default 9) that queue,
# then push their lanes and fight (simple lane AI) so one developer can form
# and actually play a full 5v5 match.
# Usage: make bots [BOTS=4] [BOTS_SERVER=127.0.0.1:4000]
BOTS ?= 9
BOTS_SERVER ?= 127.0.0.1:4000
bots: ## Join external harness bots to an existing server
	cargo run -p harness --bin bots -- --count $(BOTS) --server $(BOTS_SERVER)

# Terminate any running server, client, and bot processes started above.
stop: ## Stop matching local dev processes (broad legacy cleanup)
	-pkill -f 'cargo run -p server' 2>/dev/null || true
	-pkill -f 'cargo run -p client' 2>/dev/null || true
	-pkill -f 'cargo run -p harness --bin bots' 2>/dev/null || true
	-pkill -f 'target/debug/server' 2>/dev/null || true
	-pkill -f 'target/debug/client' 2>/dev/null || true
	-pkill -f 'target/debug/bots' 2>/dev/null || true

# Kill existing processes then start a fresh dev stack.
restart: stop ## Legacy stop, then start
	sleep 1
	$(MAKE) start

# TASK-12: live UDP matrix harness (M1/M2/M3). Requires built server binary.
verify-task-12: ## Build server and run the live UDP QA matrix
	cargo build -p server
	python3 scripts/verify_task_12_qa_matrix_live_udp.py

# Headless gameplay + matchmaking harness: builds the server, then drives it
# with bot clients over UDP to assert gameplay rules (god mode, movement
# clamp, skill gating) and release-mode match formation (TASK-22).
# No GPU, no human. Runs sequentially so spawned servers do not contend.
verify-gameplay: ## Build server and run headless gameplay/matchmaking checks
	cargo build -p server
	cargo test -p harness -- --test-threads=1
