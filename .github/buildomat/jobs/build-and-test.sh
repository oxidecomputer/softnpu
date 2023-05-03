#!/bin/bash
#:
#: name = "build-and-test"
#: variety = "basic"
#: target = "helios-latest"
#: rust_toolchain = "stable"
#: output_rules = [
#:   "/work/debug/*",
#:   "/work/release/*",
#: ]
#:
#: [[publish]]
#: series = "image"
#: name = "softnpu"
#: from_output = "/work/release/softnpu"
#:
#: [[publish]]
#: series = "image"
#: name = "softnpu.sha256.txt"
#: from_output = "/work/release/softnpu.sha256.txt"

set -o errexit
set -o pipefail
set -o xtrace

cargo --version
rustc --version

banner "check"
cargo fmt -- --check
cargo clippy -- --deny warnings

banner "build"
ptime -m cargo build
ptime -m cargo build --release

for x in debug release
do
    mkdir -p /work/$x
    cp target/$x/softnpu /work/$x/
    digest -a sha256 /work/$x/softnpu > /work/$x/softnpu.sha256.txt
done

