#!/usr/bin/env bash

RUST_LOG=debug cargo run --manifest-path ../Cargo.toml --release \
    /tmp/vfsd.sock \
    fs://?root=$PWD
