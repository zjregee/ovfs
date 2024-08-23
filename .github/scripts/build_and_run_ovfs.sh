#!/usr/bin/env bash

export KIND=fs
export ROOT=$PWD

cd ../../
RUST_LOG=debug cargo run --release
