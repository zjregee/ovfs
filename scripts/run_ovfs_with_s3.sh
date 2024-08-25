#!/usr/bin/env bash

RUST_LOG=debug cargo run --manifest-path ../Cargo.toml --release \
    /tmp/vfsd.sock \
    "s3://?bucket=test&endpoint=http://127.0.0.1:9000&access_key_id=minioadmin&secret_access_key=minioadmin&region=us-east-1"
