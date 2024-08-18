#!/usr/bin/env bash

mkdir share

export KIND=fs
export ROOT="$PWD/share"

cd ../../
RUST_LOG=debug cargo run --release
