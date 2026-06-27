// swift-tools-version: 6.0
// Builds the Parakeet STT bridge as a static library that the Rust crate
// links via build.rs. Pinned to FluidAudio v0.14.3 — bumping requires
// running the same `gh api repos/FluidInference/FluidAudio/tags` probe
// the build originally used to verify the latest stable release.

import PackageDescription

let package = Package(
    name: "ParakeetBridge",
    platforms: [
        .macOS(.v14),
    ],
    products: [
        .library(
            name: "ParakeetBridge",
            type: .static,
            targets: ["ParakeetBridge"]
        ),
    ],
    dependencies: [
        .package(url: "https://github.com/FluidInference/FluidAudio.git", exact: "0.14.3"),
    ],
    targets: [
        .target(
            name: "ParakeetBridge",
            dependencies: [
                .product(name: "FluidAudio", package: "FluidAudio"),
            ],
            path: "Sources/ParakeetBridge",
            // Pin our own bridge target to Swift 5 language mode so the
            // semaphore + Task.detached pattern (mirrors AppleFmBridge)
            // compiles without Swift 6 strict-concurrency rejections.
            // FluidAudio itself stays at its declared mode.
            swiftSettings: [
                .swiftLanguageMode(.v5),
            ]
        ),
    ]
)
