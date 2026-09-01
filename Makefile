.PHONY: gate fmt clippy test doc msrv percrate deny audit demo shot insta perf

gate: fmt clippy test doc percrate deny audit

fmt:
	cargo fmt --all --check
clippy:
	cargo clippy --workspace --all-targets --all-features -- -D warnings
test:
	cargo test --workspace
doc:
	RUSTDOCFLAGS="-D warnings" cargo doc --no-deps --workspace
msrv:
	rustup run 1.88.0 cargo check --workspace --locked --all-features
percrate:
	cargo check -p gridwatch-store
	cargo check -p gridwatch-ui
deny:
	cargo deny check
audit:
	cargo audit
demo:
	cargo run -p gridwatch --release -- run --demo
shot:
	cargo run -p gridwatch --release -- shot --format ansi
insta:
	cargo insta pending-snapshots
perf:
	scripts/perf/measure.sh
