.PHONY: server game start stop restart verify-task-12 verify-gameplay

# Run the dedicated game server (env: SERVER_ADDR, default 0.0.0.0:4000)
server:
	cargo run -p server

# Run a single game client (env: GAME_SERVER_ADDR, default 127.0.0.1:4000)
game:
	cargo run -p client

# Start the server and two clients for local single-machine multiplayer testing.
# Server and first client run in the background; second client runs in the foreground.
# Kill all three with Ctrl-C followed by `make stop`.
start:
	cargo run -p server &
	cargo run -p client &
	cargo run -p client

# Terminate any running server and client processes started by `make start`.
stop:
	-pkill -f 'cargo run -p server' 2>/dev/null || true
	-pkill -f 'cargo run -p client' 2>/dev/null || true
	-pkill -f 'target/debug/server' 2>/dev/null || true
	-pkill -f 'target/debug/client' 2>/dev/null || true

# Kill existing processes then start a fresh stack.
restart: stop
	sleep 1
	$(MAKE) start

# TASK-12: live UDP matrix harness (M1/M2/M3). Requires built server binary.
verify-task-12:
	cargo build -p server
	python3 scripts/verify_task_12_qa_matrix_live_udp.py

# Headless gameplay harness: builds the server, then drives it with bot clients
# over UDP to assert gameplay rules (god mode, movement clamp, skill gating).
# No GPU, no human. Runs sequentially so spawned servers do not contend.
verify-gameplay:
	cargo build -p server
	cargo test -p harness -- --test-threads=1
