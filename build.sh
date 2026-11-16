#!/bin/sh
# Build KlikCore and put the "KlikCore" program in this folder (run it to open the launcher).
set -e
cd "$(dirname "$0")"
cargo build --release
# Replacing the file is safe even while it runs (the old copy stays open until it exits).
cp target/release/klikcore KlikCore.new
chmod +x KlikCore.new
mv -f KlikCore.new KlikCore
echo "Built ./KlikCore - run it to open the launcher."
