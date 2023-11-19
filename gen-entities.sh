#!/bin/sh
export RUSTUP_TOOLCHAIN=nightly
sea-orm-cli generate entity -o ./entity/src --lib