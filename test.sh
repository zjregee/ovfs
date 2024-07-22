#!/usr/bin/env bash

cargo build --release

rm -rf share
mkdir share

# export KIND=fs
# export ROOT=/home/zjregee/Code/virtio/ovfs/share

export KIND=s3
export BUCKET=test
export ENDPOINT=http://127.0.0.1:9000
export ACCESS_KEY_ID=minioadmin
export SECRET_ACCESS_KEY=minioadmin
export REGION=us-east-1

RUST_LOG=debug ./target/release/ovfs
