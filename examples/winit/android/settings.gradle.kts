pluginManagement {
    repositories {
        google()
        mavenCentral()
        gradlePluginPortal()
    }
}

dependencyResolutionManagement {
    repositories {
        google()
        mavenCentral()
    }
}

rootProject.name = "pushups-winit-example"
include(":app")

// The Android module of the `pushups` crate cargo resolved for the app's Rust library, found
// through `cargo metadata`, so the Kotlin always comes from the crate version the Rust links.
val pushupsDir = run {
    val metadata = providers.exec {
        commandLine("cargo", "metadata", "--format-version", "1", "--manifest-path", file("../Cargo.toml").path)
    }.standardOutput.asText.get()
    @Suppress("UNCHECKED_CAST")
    val packages = (groovy.json.JsonSlurper().parseText(metadata) as Map<String, Any>)["packages"] as List<Map<String, Any>>
    File(packages.single { it["name"] == "pushups" }["manifest_path"] as String).parentFile
}
include(":pushups")
project(":pushups").projectDir = pushupsDir.resolve("android")
// Keeps the module's build output out of the crate's directory, which cargo's registry shares.
gradle.beforeProject {
    if (path == ":pushups") layout.buildDirectory.set(rootDir.resolve("build/pushups"))
}
