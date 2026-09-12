# ui/ - where slice 1's `.slint` files go

Empty today, and in the Slint target's freshness roots in `crates/xtask/src/smoke.rs`
already, so the first `.slint` file edited behind a built binary makes that binary
stale in the guard's eyes rather than invisible to it. Git does not store empty
directories, which is the whole reason this file exists: without it the root would not
be there on a fresh clone.
