- Strict no backward-compatibility or legacy paths no matter what.

- There is no CI. After Rust changes run `cargo fmt --all -- --check`, `cargo test`,
  `cargo test --no-default-features` and `cargo clippy --all-targets -- -D warnings`, on stable.
- The crate forbids unsafe code (`#![forbid(unsafe_code)]` in `src/lib.rs`).
- Decode performance is measured as `docs/benchmarks/README.md` describes.
