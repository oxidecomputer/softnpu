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
#:
#: [[publish]]
#: series = "image"
#: name = "npuzone"
#: from_output = "/work/release/npuzone"
#:
#: [[publish]]
#: series = "image"
#: name = "npuzone.sha256.txt"
#: from_output = "/work/release/npuzone.sha256.txt"
#:
#: [[publish]]
#: series = "image"
#: name = "npuvm"
#: from_output = "/work/release/npuvm"
#:
#: [[publish]]
#: series = "image"
#: name = "npuvm.sha256.txt"
#: from_output = "/work/release/npuvm.sha256.txt"

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
    cp target/$x/npuzone /work/$x/
    cp target/$x/npuvm /work/$x/
    digest -a sha256 /work/$x/softnpu > /work/$x/softnpu.sha256.txt
    digest -a sha256 /work/$x/npuzone > /work/$x/npuzone.sha256.txt
    digest -a sha256 /work/$x/npuvm > /work/$x/npuvm.sha256.txt
done

