#!/bin/bash
export PKG_CONFIG_ALLOW_CROSS=1
export OPENSSL_STATIC=true
export OPENSSL_DIR=/home/chris/downloads/musl
cargo build --release --target x86_64-unknown-linux-musl
