.PHONY: server server-dev game game2d start start-release play-bots bots stop restart verify-task-12 verify-gameplay

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
# Env vars: OMOBA_MATCH_MODE=release|dev, OMOBA_TEAM_SIZE=<players per team,
# default 5>, SERVER_ADDR (server bind), GAME_SERVER_ADDR (client target),
# OMOBA_AUTOJOIN=<class>:<avatar|->:<team> (client joins without UI).
# ---------------------------------------------------------------------------

GAME_SERVER_ADDR ?= 127.0.0.1:4000

# Run the game server in RELEASE match mode (matches form to 5v5 before starting).
server:
	cargo run -p server

# Run the game server in DEV match mode (first join starts the match immediately).
server-dev:
	OMOBA_MATCH_MODE=dev cargo run -p server

# Run a single game client (env: GAME_SERVER_ADDR, default 127.0.0.1:4000)
game:
	GAME_SERVER_ADDR=$(GAME_SERVER_ADDR) cargo run -p client

# Run one client in the genuine orthographic XY renderer.  The inline mode
# assignment intentionally overrides any conflicting caller environment.
# GAME_SERVER_ADDR is inherited exactly like `make game`.
game2d:
	GAME_SERVER_ADDR=$(GAME_SERVER_ADDR) OMOBA_PLAYER_VISUAL_MODE=sprite2d cargo run -p client

# DEV quick-start: dev-mode server and two clients for single-machine testing.
# Instant match start, no matchmaking gate. Server and first client run in the
# background; second client runs in the foreground. Clean up with `make stop`.
start:
	OMOBA_MATCH_MODE=dev cargo run -p server &
	cargo run -p client &
	cargo run -p client

# RELEASE-like start: release-mode server in the background plus one client in
# the foreground. The client waits in matchmaking until the roster is full —
# fill the remaining seats with `make bots` from another terminal.
start-release:
	cargo run -p server &
	cargo run -p client

# One-command full-match demo for a single developer: release-mode server and
# nine fill bots in the background, your client in the foreground. Pick a
# class/avatar/team in the client: the match forms to 5v5 and starts through
# the real matchmaking flow. Clean up with `make stop`.
play-bots:
	cargo run -p server &
	cargo build -p server
	sleep 2
	cargo run -p harness --bin bots -- --count 9 &
	cargo run -p client

# Fill bots for a running server: joins BOTS players (default 9) that queue,
# then push their lanes and fight (simple lane AI) so one developer can form
# and actually play a full 5v5 match.
# Usage: make bots [BOTS=4] [BOTS_SERVER=127.0.0.1:4000]
BOTS ?= 9
BOTS_SERVER ?= 127.0.0.1:4000
bots:
	cargo run -p harness --bin bots -- --count $(BOTS) --server $(BOTS_SERVER)

# Terminate any running server, client, and bot processes started above.
stop:
	-pkill -f 'cargo run -p server' 2>/dev/null || true
	-pkill -f 'cargo run -p client' 2>/dev/null || true
	-pkill -f 'cargo run -p harness --bin bots' 2>/dev/null || true
	-pkill -f 'target/debug/server' 2>/dev/null || true
	-pkill -f 'target/debug/client' 2>/dev/null || true
	-pkill -f 'target/debug/bots' 2>/dev/null || true

# Kill existing processes then start a fresh dev stack.
restart: stop
	sleep 1
	$(MAKE) start

# TASK-12: live UDP matrix harness (M1/M2/M3). Requires built server binary.
verify-task-12:
	cargo build -p server
	python3 scripts/verify_task_12_qa_matrix_live_udp.py

# Headless gameplay + matchmaking harness: builds the server, then drives it
# with bot clients over UDP to assert gameplay rules (god mode, movement
# clamp, skill gating) and release-mode match formation (TASK-22).
# No GPU, no human. Runs sequentially so spawned servers do not contend.
verify-gameplay:
	cargo build -p server
	cargo test -p harness -- --test-threads=1
