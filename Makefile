.PHONY: server game start stop restart

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
