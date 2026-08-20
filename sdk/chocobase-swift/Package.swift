// swift-tools-version: 5.7
import PackageDescription

let package = Package(
    name: "ChocoBase",
    platforms: [
        .iOS(.v14),
        .macOS(.v11),
        .tvOS(.v14),
        .watchOS(.v7)
    ],
    products: [
        .library(
            name: "ChocoBase",
            targets: ["ChocoBase"]
        ),
    ],
    dependencies: [],
    targets: [
        .target(
            name: "ChocoBase",
            dependencies: []
        ),
    ]
)
