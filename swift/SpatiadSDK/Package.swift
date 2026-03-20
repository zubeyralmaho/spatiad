// swift-tools-version: 5.9

import PackageDescription

let package = Package(
    name: "SpatiadSDK",
    platforms: [
        .iOS(.v15),
        .macOS(.v12),
    ],
    products: [
        .library(name: "SpatiadSDK", targets: ["SpatiadSDK"]),
    ],
    targets: [
        .target(name: "SpatiadSDK"),
    ]
)
