.PHONY: server game start

server:
	cargo run -p server

game:
	cargo run -p client

start:
	cargo run -p server &
	cargo run -p client &
	cargo run -p client
