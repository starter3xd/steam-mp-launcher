#!/usr/bin/env bash
# Build the Rust backend + C++/Qt UI.  KDE projects should use CMake; qmake
# remains as a fallback for systems without cmake.
set -euo pipefail
cd "$(dirname -- "$0")"
export PATH="$HOME/.cargo/bin:$PATH"

if command -v cmake >/dev/null 2>&1; then
	cmake -S . -B build -DCMAKE_BUILD_TYPE=Release
	cmake --build build --parallel
else
	cargo build --release --manifest-path backend/Cargo.toml
	(
		cd ui
		qmake6 ui.pro
		make -j"$(nproc)"
	)
fi
